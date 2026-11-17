//! REMASTER: the Enhanced engine's "air" layer (#494 phase 4): noise with a recording's own colour and pulse.
//! A short-time Fourier transform of the clip keeps every frame's magnitudes (5.8 ms at 22 050 Hz) and draws new
//! phases, so the engine's harmonic lines turn into noise while its band colour and its frame-to-frame level (the
//! firing pulse) stay. The frames wrap around the clip, so the result has the clip's length and loops as the clip
//! does; played beside it at the same pitch from the same start, it pulses in step with it.

use std::f64::consts::TAU;

/// Frame length in samples (a power of two).
const FRAME: usize = 128;
/// Hop between frames: half a frame (sqrt-Hann analysis and synthesis windows sum to one).
const HOP: usize = FRAME / 2;

/// In-place radix-2 FFT of (`re`, `im`), length `FRAME`; `inverse` without the 1/n.
fn fft(re: &mut [f64; FRAME], im: &mut [f64; FRAME], inverse: bool) {
    let n = FRAME;
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        for k in 0..len / 2 {
            let a = sign * TAU * k as f64 / len as f64;
            let (wr, wi) = (a.cos(), a.sin());
            for s in (0..n).step_by(len) {
                let (p, q) = (s + k, s + k + len / 2);
                let (tr, ti) = (re[q] * wr - im[q] * wi, re[q] * wi + im[q] * wr);
                (re[q], im[q]) = (re[p] - tr, im[p] - ti);
                re[p] += tr;
                im[p] += ti;
            }
        }
        len <<= 1;
    }
}

/// Noise with the colour and the pulse of `src` (a loop at `rate` Hz), its content below `low_cut_hz` left out.
/// Same length as `src`, looping seamlessly when `src` does; scaled to the RMS of `src`'s own content above the
/// cut, so a layer at 0 dB adds as much noise as the recording carries there. Deterministic for a `seed`.
pub fn noise_like(src: &[f32], rate: u32, low_cut_hz: f32, seed: u32) -> Vec<f32> {
    let n = src.len();
    if n < FRAME {
        return vec![0.0; n];
    }
    let win: Vec<f64> = (0..FRAME).map(|i| (0.5 - 0.5 * (TAU * i as f64 / FRAME as f64).cos()).sqrt()).collect();
    let cut = (low_cut_hz.max(0.0) as f64 * FRAME as f64 / rate.max(1) as f64).ceil() as usize;
    let mut rng = seed;
    let mut phase = || {
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (rng >> 8) as f64 / (1u32 << 24) as f64 * TAU
    };
    // the source's band above the cut (its own phases), the noise, and how much window lies on each sample
    let (mut band, mut air, mut wsum) = (vec![0f64; n], vec![0f64; n], vec![0f64; n]);
    for s0 in (0..n).step_by(HOP) {
        let (mut re, mut im) = ([0f64; FRAME], [0f64; FRAME]);
        for i in 0..FRAME {
            re[i] = src[(s0 + i) % n] as f64 * win[i];
        }
        fft(&mut re, &mut im, false);
        for k in 0..FRAME {
            if k.min(FRAME - k) < cut {
                (re[k], im[k]) = (0.0, 0.0);
            }
        }
        let (mut br, mut bi) = (re, im);
        fft(&mut br, &mut bi, true);
        // new phases, conjugate-symmetric so the frame stays real; DC and Nyquist keep their magnitude, a random sign
        for k in 0..=FRAME / 2 {
            let m = re[k].hypot(im[k]);
            let a = phase();
            if k == 0 || k == FRAME / 2 {
                (re[k], im[k]) = (if a < std::f64::consts::PI { m } else { -m }, 0.0);
            } else {
                (re[k], im[k]) = (m * a.cos(), m * a.sin());
                (re[FRAME - k], im[FRAME - k]) = (re[k], -im[k]);
            }
        }
        fft(&mut re, &mut im, true);
        for i in 0..FRAME {
            let j = (s0 + i) % n;
            band[j] += br[i] / FRAME as f64 * win[i];
            air[j] += re[i] / FRAME as f64 * win[i];
            wsum[j] += win[i] * win[i];
        }
    }
    // the band adds up coherently (divide by the window sum), the noise frames in power (by its square root)
    let (mut eb, mut ea) = (0.0, 0.0);
    for j in 0..n {
        let w = wsum[j].max(1e-9);
        band[j] /= w;
        air[j] /= w.sqrt();
        eb += band[j] * band[j];
        ea += air[j] * air[j];
    }
    let k = if ea > 0.0 { (eb / ea).sqrt() } else { 0.0 };
    air.iter().map(|&v| (v * k) as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    fn lcg_noise(n: usize, mut seed: u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 16) as f32 / 32768.0 - 1.0
            })
            .collect()
    }

    /// amplitude of `freq` in `x` (Hann-windowed single-bin DFT)
    fn line(x: &[f32], freq: f64, rate: f64) -> f64 {
        let n = x.len();
        let (mut re, mut im, mut ws) = (0.0, 0.0, 0.0);
        for (i, &v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos();
            let a = TAU * freq * i as f64 / rate;
            re += w * v as f64 * a.cos();
            im -= w * v as f64 * a.sin();
            ws += w;
        }
        2.0 * (re * re + im * im).sqrt() / ws
    }

    #[test]
    fn fft_round_trip() {
        let x = lcg_noise(FRAME, 7);
        let (mut re, mut im) = ([0f64; FRAME], [0f64; FRAME]);
        for i in 0..FRAME {
            re[i] = x[i] as f64;
        }
        fft(&mut re, &mut im, false);
        // bin 0 is the sum
        assert!((re[0] - x.iter().map(|&v| v as f64).sum::<f64>()).abs() < 1e-9);
        fft(&mut re, &mut im, true);
        for i in 0..FRAME {
            assert!((re[i] / FRAME as f64 - x[i] as f64).abs() < 1e-9 && im[i].abs() < 1e-9);
        }
    }

    #[test]
    fn a_tone_becomes_noise_of_its_band() {
        // 1 kHz for exactly 1000 periods (a seamless loop) and a little noise
        let n = 22050;
        let nz = lcg_noise(n, 3);
        let src: Vec<f32> = (0..n).map(|i| 0.5 * (TAU * 1000.0 * i as f64 / 22050.0).sin() as f32 + 0.025 * nz[i]).collect();
        let air = noise_like(&src, 22050, 600.0, 1);
        assert_eq!(air.len(), n);
        // the tone is above the cut: the noise carries its level
        assert!((rms(&air) / rms(&src) - 1.0).abs() < 0.03, "rms {} vs {}", rms(&air), rms(&src));
        // the line is spread: its single-bin amplitude drops by more than 20 dB
        let db = 20.0 * (line(&air, 1000.0, 22050.0) / line(&src, 1000.0, 22050.0)).log10();
        assert!(db < -20.0, "line {db} dB");
        // ... into its own band: most of the energy stays within 0.5..2 kHz
        let nfft = 16384;
        let (mut e_in, mut e_all) = (0.0, 0.0);
        for k in 1..nfft / 2 {
            let f = k as f64 * 22050.0 / nfft as f64;
            let a = line(&air[..nfft], f, 22050.0).powi(2);
            if k % 8 == 0 {
                e_all += a;
                if (500.0..2000.0).contains(&f) {
                    e_in += a;
                }
            }
        }
        assert!(e_in / e_all > 0.85, "share in 0.5-2 kHz {}", e_in / e_all);
        // it loops: the wrap is an ordinary step of the noise
        let mut st: Vec<f32> = air.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        st.sort_by(|a, b| a.total_cmp(b));
        assert!((air[n - 1] - air[0]).abs() <= st[st.len() * 99 / 100], "wrap {}", (air[n - 1] - air[0]).abs());
        // seeded
        assert_eq!(air, noise_like(&src, 22050, 600.0, 1));
        assert_ne!(air, noise_like(&src, 22050, 600.0, 2));
    }

    #[test]
    fn the_pulse_stays() {
        // noise pulsing at 40 Hz (exactly 40 periods): the layer pulses with it
        let n = 22050;
        let nz = lcg_noise(n, 9);
        let env: Vec<f64> = (0..n).map(|i| 1.0 + 0.9 * (TAU * 40.0 * i as f64 / 22050.0).sin()).collect();
        let src: Vec<f32> = (0..n).map(|i| (nz[i] as f64 * env[i] * 0.3) as f32).collect();
        let air = noise_like(&src, 22050, 600.0, 1);
        // RMS in 5 ms windows against the pulse
        let w = 110;
        let lv: Vec<f64> = air.chunks_exact(w).map(rms).collect();
        let ev: Vec<f64> = env.chunks_exact(w).map(|c| c.iter().sum::<f64>() / w as f64).collect();
        let m = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let (ml, me) = (m(&lv), m(&ev));
        let cov: f64 = lv.iter().zip(&ev).map(|(a, b)| (a - ml) * (b - me)).sum();
        let (vl, ve): (f64, f64) = (lv.iter().map(|a| (a - ml).powi(2)).sum(), ev.iter().map(|b| (b - me).powi(2)).sum());
        let corr = cov / (vl * ve).sqrt();
        assert!(corr > 0.8, "pulse correlation {corr}");
    }
}
