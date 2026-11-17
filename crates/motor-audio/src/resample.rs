//! REMASTER: band-limited resampling and loop seams for the Enhanced engine sound (#494 phase 1).
//! The game's own mixer path is the linear interpolation in `Mixer::render` (CLASSIC); a clip
//! marked `Interp::Sinc` is read through this windowed-sinc kernel instead, and `enhanced_loop`
//! makes a clip's wrap-around seamless. Nothing here touches a clip that is not asked for.

use crate::{Clip, Interp};
use std::sync::{Arc, OnceLock};

/// Zero crossings of the kernel on each side of the read position (at a cutoff of the source's Nyquist).
const HALF: usize = 12;
/// Table points per zero crossing (the kernel between points is read by linear interpolation).
const RES: usize = 256;
/// Kaiser window shape (about -75 dB stopband).
const BETA: f64 = 8.0;
/// Share of the usable band kept: the kernel's cutoff is this x the lower of the source's and the output's Nyquist.
const PASS: f32 = 0.95;

/// Modified Bessel function of the first kind, order 0 (power series).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..60 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
        if term < 1e-14 * sum {
            break;
        }
    }
    sum
}

/// `sinc(x) x kaiser(x / HALF)` for x = i / RES zero crossings; one spare entry for the interpolation.
fn table() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        let norm = bessel_i0(BETA);
        (0..=HALF * RES + 1)
            .map(|i| {
                let x = i as f64 / RES as f64;
                let u = x / HALF as f64;
                if u >= 1.0 {
                    return 0.0;
                }
                let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
                (sinc * bessel_i0(BETA * (1.0 - u * u).sqrt()) / norm) as f32
            })
            .collect()
    })
}

/// The kernel's cutoff (a fraction of the source's Nyquist) for a voice that advances `step` source
/// samples per output frame: below 1 the output's own Nyquist is the lower, so the kernel narrows.
pub fn cutoff(step: f64) -> f32 {
    if step > 1.0 {
        PASS / step as f32
    } else {
        PASS
    }
}

/// The band-limited value of `data` at the fractional position `pos` (in samples), cutoff `fc`
/// from `cutoff`. `looping` = the loop region (start, end): reads past its end wrap into it, as the
/// linear path does; without it the clip is silent outside its ends.
pub fn sinc_at(data: &[f32], pos: f64, fc: f32, looping: Option<(usize, usize)>) -> f32 {
    let t = table();
    let fc = fc.clamp(0.05, 1.0) as f64;
    let reach = HALF as f64 / fc;
    let (first, last) = ((pos - reach).ceil() as i64, (pos + reach).floor() as i64);
    let n = data.len() as i64;
    let fetch = |j: i64| -> f32 {
        match looping {
            Some((ls, le)) => {
                let (ls, le) = (ls as i64, (le as i64).min(n));
                if j >= le && le > ls {
                    data[(ls + (j - ls).rem_euclid(le - ls)) as usize]
                } else if j < 0 {
                    // before the clip: the loop's tail when the loop starts at 0 (the seam), else silence
                    if ls == 0 && le > 0 { data[(j.rem_euclid(le)) as usize] } else { 0.0 }
                } else {
                    data[j as usize]
                }
            }
            None => if (0..n).contains(&j) { data[j as usize] } else { 0.0 },
        }
    };
    let mut acc = 0.0f64;
    for j in first..=last {
        let x = (j as f64 - pos).abs() * fc * RES as f64;
        let (k, f) = (x as usize, x.fract());
        let w = t[k] as f64 * (1.0 - f) + t[k + 1] as f64 * f;
        acc += w * fetch(j) as f64;
    }
    (acc * fc) as f32
}

/// A seamless loop of `data`: the loop length L is the one in the last `search` samples whose
/// continuation `data[L..L+x]` best matches the start `data[0..x]` (normalised cross-correlation);
/// the first `x` samples then crossfade from the tail's own continuation into the start, so the
/// wrap `out[L-1] -> out[0]` continues the recording instead of jumping. Returns L samples.
pub fn seamless_loop(data: &[f32], xfade: usize, search: usize) -> Vec<f32> {
    let n = data.len();
    let x = xfade.min(n / 4);
    if x < 2 {
        return data.to_vec();
    }
    let lo = n.saturating_sub(search).max(n / 2);
    let hi = n - x;
    let head = &data[..x];
    let e_head: f64 = head.iter().map(|&v| (v as f64).powi(2)).sum();
    let mut best = (hi, f64::MIN);
    for l in lo..=hi {
        let tail = &data[l..l + x];
        let (mut dot, mut e) = (0.0f64, 0.0f64);
        for (a, b) in head.iter().zip(tail) {
            dot += *a as f64 * *b as f64;
            e += (*b as f64).powi(2);
        }
        let score = dot / (e * e_head).sqrt().max(1e-12);
        if score > best.1 {
            best = (l, score);
        }
    }
    let l = best.0;
    let mut out = data[..l].to_vec();
    for i in 0..x {
        // raised cosine 0 -> 1 over the fade: the start comes in as the tail's continuation goes out
        let w_in = 0.5 - 0.5 * (std::f32::consts::PI * (i as f32 + 0.5) / x as f32).cos();
        out[i] = data[i] * w_in + data[l + i] * (1.0 - w_in);
    }
    out
}

/// REMASTER: the Enhanced engine's version of a loop clip: a seamless loop of the whole clip (the
/// clips that carry their own loop points keep them) read with `Interp::Sinc`.
pub fn enhanced_loop(clip: &Clip, xfade_ms: f32, search_ms: f32) -> Clip {
    let ms = |v: f32| (v.max(0.0) * clip.rate as f32 / 1000.0) as usize;
    let data = if clip.loop_points.is_none() { Arc::new(seamless_loop(&clip.data, ms(xfade_ms), ms(search_ms))) } else { clip.data.clone() };
    Clip { rate: clip.rate, data, loop_points: clip.loop_points, interp: Interp::Sinc, glide: clip.glide }
}
