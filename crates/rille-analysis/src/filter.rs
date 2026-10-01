//! Offline biquad filters (f64 state) and zero-phase helpers for analysis.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    pub fn lowpass(sr: f64, f: f64, q: f64) -> Self {
        let w = 2.0 * PI * f / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        Self::normalized((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn highpass(sr: f64, f: f64, q: f64) -> Self {
        let w = 2.0 * PI * f / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        Self::normalized((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    /// High shelf (RBJ), gain in dB.
    pub fn high_shelf(sr: f64, f: f64, q: f64, gain_db: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * f / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        let sa = 2.0 * a.sqrt() * alpha;
        Self::normalized(
            a * ((a + 1.0) + (a - 1.0) * c + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - sa),
            (a + 1.0) - (a - 1.0) * c + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - sa,
        )
    }

    /// Filters `x` in place (transposed direct form II).
    pub fn run(&self, x: &mut [f32]) {
        let (mut z1, mut z2) = (0.0f64, 0.0f64);
        for v in x.iter_mut() {
            let i = f64::from(*v);
            let o = self.b0 * i + z1;
            z1 = self.b1 * i - self.a1 * o + z2;
            z2 = self.b2 * i - self.a2 * o;
            *v = o as f32;
        }
    }
}

/// Forward-backward filtering: squared magnitude response, zero phase, so
/// transient positions are not shifted.
pub fn filtfilt(filters: &[Biquad], x: &mut [f32]) {
    for f in filters {
        f.run(x);
    }
    x.reverse();
    for f in filters {
        f.run(x);
    }
    x.reverse();
}

/// Zero-phase low-pass followed by decimation by `factor`.
pub fn decimate(x: &[f32], sr: f64, factor: usize) -> Vec<f32> {
    let mut y = x.to_vec();
    let cutoff = 0.4 * sr / factor as f64;
    let lp = Biquad::lowpass(sr, cutoff, std::f64::consts::FRAC_1_SQRT_2);
    filtfilt(&[lp, lp], &mut y);
    y.iter().step_by(factor).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(sr: f64, f: f64, n: usize) -> Vec<f32> {
        (0..n).map(|i| (2.0 * PI * f * i as f64 / sr).sin() as f32).collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn lowpass_attenuates() {
        let sr = 44_100.0;
        let lp = Biquad::lowpass(sr, 200.0, std::f64::consts::FRAC_1_SQRT_2);
        let mut lo = sine(sr, 50.0, 44_100);
        let mut hi = sine(sr, 5000.0, 44_100);
        filtfilt(&[lp], &mut lo);
        filtfilt(&[lp], &mut hi);
        assert!(rms(&lo[4000..40_000]) > 0.68);
        assert!(rms(&hi[4000..40_000]) < 0.001);
    }

    #[test]
    fn filtfilt_keeps_transient_position() {
        let mut x = vec![0.0f32; 4000];
        x[2000] = 1.0;
        filtfilt(&[Biquad::lowpass(44_100.0, 500.0, 0.707)], &mut x);
        let peak = x.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(peak, 2000);
    }
}
