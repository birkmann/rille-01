//! Fractional-position reads from a stereo source, for varispeed playback.
//!
//! Frames outside the source slice read as silence, so callers can read
//! across the start and end of a track without special cases.

use std::f64::consts::PI;

/// Lowest supported cutoff factor (reading at up to 8x speed).
const MIN_CUTOFF: f32 = 0.125;

/// Kaiser-windowed sinc interpolator with a precomputed kernel table.
///
/// The table holds one side of the (symmetric) kernel at `phases` points per
/// source frame and is linearly interpolated. A `cutoff` below 1 stretches the
/// kernel (more taps) and lowers its corner to `cutoff * Nyquist`, which is
/// the anti-aliasing needed when reading faster than real time.
#[derive(Clone, Debug)]
pub struct SincTable {
    taps: usize,
    phases: usize,
    half: f32,
    table: Box<[f32]>,
}

impl SincTable {
    /// `taps` is the kernel length at cutoff 1 (rounded up to even, at least
    /// 4; 16 or 32 are typical), `phases` the table resolution per frame.
    pub fn new(taps: usize, phases: usize) -> Self {
        let taps = (taps.max(4) + 1) & !1;
        let phases = phases.max(16);
        let half = taps / 2;
        // Stop-band ~90 dB for 32 taps; shorter kernels trade it for a narrower transition band.
        let beta = if taps >= 32 { 9.0 } else { 6.5 };
        let n = half * phases;
        let table = (0..=n + 1)
            .map(|i| {
                if i >= n {
                    return 0.0;
                }
                let t = i as f64 / phases as f64;
                let sinc = if i == 0 { 1.0 } else { (PI * t).sin() / (PI * t) };
                let r = t / half as f64;
                (sinc * bessel_i0(beta * (1.0 - r * r).sqrt()) / bessel_i0(beta)) as f32
            })
            .collect();
        Self { taps, phases, half: half as f32, table }
    }

    pub fn taps(&self) -> usize {
        self.taps
    }

    /// Cutoff factor for reading `ratio` source frames per output frame.
    pub fn cutoff_for_ratio(ratio: f64) -> f32 {
        let r = ratio.abs();
        if r > 1.0 { (1.0 / r) as f32 } else { 1.0 }
    }

    /// Kernel value at `t` source frames from the centre (cutoff 1).
    #[inline]
    fn kernel(&self, t: f32) -> f32 {
        let x = t.abs() * self.phases as f32;
        let i = x as usize;
        if i + 1 >= self.table.len() {
            return 0.0;
        }
        let a = self.table[i];
        a + (x - i as f32) * (self.table[i + 1] - a)
    }

    /// Band-limited read of `src` at fractional frame `pos`. `cutoff` in
    /// `0.125..=1` (see [`cutoff_for_ratio`](Self::cutoff_for_ratio)).
    pub fn sample(&self, src: &[[f32; 2]], pos: f64, cutoff: f32) -> [f32; 2] {
        if !pos.is_finite() || src.is_empty() {
            return [0.0; 2];
        }
        let c = if cutoff >= MIN_CUTOFF { cutoff.min(1.0) } else { MIN_CUTOFF }; // NaN -> minimum
        let reach = (self.half / c) as f64;
        let first = (pos - reach).ceil().max(0.0);
        let last = (pos + reach).floor().min(src.len() as f64 - 1.0);
        if first > last {
            return [0.0; 2];
        }
        let first = first as usize;
        let mut acc = [0.0f32; 2];
        for (i, s) in src[first..=last as usize].iter().enumerate() {
            let w = c * self.kernel(c * (pos - (first + i) as f64) as f32);
            acc[0] += w * s[0];
            acc[1] += w * s[1];
        }
        acc
    }
}

/// Modified Bessel function of the first kind, order 0 (power series).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// 4-point cubic Hermite (Catmull-Rom) read of `src` at fractional frame `pos`.
pub fn cubic_hermite(src: &[[f32; 2]], pos: f64) -> [f32; 2] {
    if !pos.is_finite() {
        return [0.0; 2];
    }
    let i = pos.floor();
    let t = (pos - i) as f32;
    let at = |k: f64| -> [f32; 2] { if k >= 0.0 && k < src.len() as f64 { src[k as usize] } else { [0.0; 2] } };
    let (ym1, y0, y1, y2) = (at(i - 1.0), at(i), at(i + 1.0), at(i + 2.0));
    let mut out = [0.0; 2];
    for ch in 0..2 {
        let c1 = 0.5 * (y1[ch] - ym1[ch]);
        let c2 = ym1[ch] - 2.5 * y0[ch] + 2.0 * y1[ch] - 0.5 * y2[ch];
        let c3 = 0.5 * (y2[ch] - ym1[ch]) + 1.5 * (y0[ch] - y1[ch]);
        out[ch] = ((c3 * t + c2) * t + c1) * t + y0[ch];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{sine, tone_amplitude};

    const SR: f32 = 48_000.0;

    /// RMS error in dB (re. full-scale sine) of reading a 1-amplitude sine at
    /// `pos = offset + n` against the exact value.
    fn error_db(read: impl Fn(&[[f32; 2]], f64) -> [f32; 2], freq: f64, offset: f64) -> f64 {
        let src = sine(SR, freq, 1.0, 20_000);
        let mut err = 0.0;
        let n = 10_000;
        for k in 5000..5000 + n {
            let pos = k as f64 + offset;
            let want = (std::f64::consts::TAU * freq * pos / SR as f64).sin();
            err += (read(&src, pos)[0] as f64 - want).powi(2);
        }
        10.0 * (err / n as f64 / 0.5).log10()
    }

    #[test]
    fn sinc_ratio_one_reproduces_input() {
        let t = SincTable::new(32, 256);
        let src = sine(SR, 1000.0, 0.8, 4000);
        for k in 0..src.len() {
            let y = t.sample(&src, k as f64, 1.0);
            assert!((y[0] - src[k][0]).abs() < 1e-6);
        }
        for (freq, offset) in [(1000.0, 0.37), (5000.0, 0.5), (12_000.0, 0.123)] {
            let e = error_db(|s, p| t.sample(s, p, 1.0), freq, offset);
            assert!(e < -80.0, "{freq} Hz @ +{offset}: {e} dB");
        }
    }

    #[test]
    fn sinc_downsampling_rejects_aliases() {
        let t = SincTable::new(32, 256);
        let ratio = 1.5;
        let cutoff = SincTable::cutoff_for_ratio(ratio);
        let read = |freq: f64| {
            let src = sine(SR, freq, 1.0, 120_000);
            let out: Vec<[f32; 2]> = (0..48_000).map(|n| t.sample(&src, 10_000.0 + n as f64 * ratio, cutoff)).collect();
            let rms = (out.iter().map(|f| (f[0] as f64).powi(2)).sum::<f64>() / out.len() as f64).sqrt();
            (out, 20.0 * (rms * 2f64.sqrt()).log10())
        };
        // Above the output Nyquist (16 kHz in source terms): must not alias.
        for freq in [19_000.0, 21_000.0, 23_000.0] {
            let (_, db) = read(freq);
            assert!(db < -60.0, "{freq} Hz alias at {db} dB");
        }
        // Passband tone comes through at the scaled frequency.
        let (out, _) = read(2000.0);
        let amp = tone_amplitude(SR, 2000.0 * ratio, &out, 0);
        assert!((20.0 * amp.log10()).abs() < 0.05, "{amp}");
    }

    #[test]
    fn out_of_range_is_silence() {
        let t = SincTable::new(16, 256);
        let src = vec![[1.0f32, 1.0]; 100];
        assert_eq!(t.sample(&src, -50.0, 1.0), [0.0; 2]);
        assert_eq!(t.sample(&src, 1e12, 0.5), [0.0; 2]);
        assert_eq!(t.sample(&src, f64::NAN, 1.0), [0.0; 2]);
        assert!(t.sample(&src, -0.5, 1.0)[0] > 0.3); // half-overlapping the start
        assert!((t.sample(&src, 50.3, 0.5)[0] - 1.0).abs() < 1e-3);
        assert_eq!(cubic_hermite(&src, -5.0), [0.0; 2]);
        assert_eq!(cubic_hermite(&src, f64::INFINITY), [0.0; 2]);
        assert_no_alloc::assert_no_alloc(|| t.sample(&src, 10.5, 0.2));
    }

    #[test]
    fn hermite_accuracy() {
        let src = sine(SR, 100.0, 1.0, 16);
        for (k, f) in src.iter().enumerate() {
            assert_eq!(cubic_hermite(&src, k as f64), *f);
        }
        let e = error_db(cubic_hermite, 200.0, 0.5);
        assert!(e < -80.0, "{e}");
        let e = error_db(cubic_hermite, 2000.0, 0.37);
        assert!(e < -40.0, "{e}");
    }
}
