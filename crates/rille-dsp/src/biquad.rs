//! Biquad filters with RBJ "Audio EQ Cookbook" coefficients, run in
//! transposed direct form II. Coefficients are computed in f64 and stored
//! normalised (`a0 = 1`).

use crate::denormal::undenormal;
use std::f64::consts::PI;

/// Normalised biquad coefficients.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Default for Coeffs {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Coeffs {
    /// Passes the signal unchanged.
    pub const IDENTITY: Self = Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    pub fn lowpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos, alpha) = prewarp(sample_rate, freq, q);
        let b = (1.0 - cos) / 2.0;
        Self::raw([b, 2.0 * b, b], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    pub fn highpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos, alpha) = prewarp(sample_rate, freq, q);
        let b = (1.0 + cos) / 2.0;
        Self::raw([b, -2.0 * b, b], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    /// Band-pass with 0 dB peak gain.
    pub fn bandpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos, alpha) = prewarp(sample_rate, freq, q);
        Self::raw([alpha, 0.0, -alpha], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    pub fn allpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos, alpha) = prewarp(sample_rate, freq, q);
        Self::raw([1.0 - alpha, -2.0 * cos, 1.0 + alpha], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    pub fn peaking(sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let (cos, alpha) = prewarp(sample_rate, freq, q);
        let a = shelf_a(gain_db);
        Self::raw([1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a], [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a])
    }

    pub fn lowshelf(sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let (cos, alpha) = prewarp(sample_rate, freq, q);
        let a = shelf_a(gain_db);
        let s = 2.0 * a.sqrt() * alpha;
        Self::raw(
            [
                a * ((a + 1.0) - (a - 1.0) * cos + s),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - s),
            ],
            [(a + 1.0) + (a - 1.0) * cos + s, -2.0 * ((a - 1.0) + (a + 1.0) * cos), (a + 1.0) + (a - 1.0) * cos - s],
        )
    }

    pub fn highshelf(sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let (cos, alpha) = prewarp(sample_rate, freq, q);
        let a = shelf_a(gain_db);
        let s = 2.0 * a.sqrt() * alpha;
        Self::raw(
            [
                a * ((a + 1.0) + (a - 1.0) * cos + s),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - s),
            ],
            [(a + 1.0) - (a - 1.0) * cos + s, 2.0 * ((a - 1.0) - (a + 1.0) * cos), (a + 1.0) - (a - 1.0) * cos - s],
        )
    }

    fn raw(b: [f64; 3], a: [f64; 3]) -> Self {
        let n = 1.0 / a[0];
        Self {
            b0: (b[0] * n) as f32,
            b1: (b[1] * n) as f32,
            b2: (b[2] * n) as f32,
            a1: (a[1] * n) as f32,
            a2: (a[2] * n) as f32,
        }
    }

    /// Magnitude response (linear) at `freq`.
    pub fn magnitude(&self, sample_rate: f32, freq: f32) -> f32 {
        let w = 2.0 * PI * freq as f64 / sample_rate as f64;
        // H(e^jw) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2)
        let eval = |c0: f64, c1: f64, c2: f64| {
            let re = c0 + c1 * w.cos() + c2 * (2.0 * w).cos();
            let im = -c1 * w.sin() - c2 * (2.0 * w).sin();
            re.hypot(im)
        };
        let num = eval(self.b0 as f64, self.b1 as f64, self.b2 as f64);
        let den = eval(1.0, self.a1 as f64, self.a2 as f64);
        (num / den) as f32
    }
}

/// `cos(w0)` and `alpha` for the cookbook formulas; clamps the frequency to
/// `(0, 0.49 * sample_rate)` and Q to a positive value so the result is stable.
fn prewarp(sample_rate: f32, freq: f32, q: f32) -> (f64, f64) {
    let sr = sample_rate as f64;
    let f = (freq as f64).max(1e-3).min(0.49 * sr); // max() also maps NaN to the minimum
    let w0 = 2.0 * PI * f / sr;
    let q = (q as f64).max(1e-3);
    (w0.cos(), w0.sin() / (2.0 * q))
}

/// `A` of the cookbook formulas; non-finite gains count as 0 dB.
fn shelf_a(gain_db: f32) -> f64 {
    if gain_db.is_finite() { 10f64.powf(gain_db as f64 / 40.0) } else { 1.0 }
}

/// Mono biquad (transposed direct form II).
#[derive(Clone, Debug, Default)]
pub struct Biquad {
    c: Coeffs,
    s1: f32,
    s2: f32,
}

impl Biquad {
    pub fn new(c: Coeffs) -> Self {
        Self { c, s1: 0.0, s2: 0.0 }
    }

    /// Replaces the coefficients, keeping the state.
    pub fn set_coeffs(&mut self, c: Coeffs) {
        self.c = c;
    }

    pub fn coeffs(&self) -> Coeffs {
        self.c
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let c = &self.c;
        let y = c.b0 * x + self.s1;
        self.s1 = undenormal(c.b1 * x - c.a1 * y + self.s2);
        self.s2 = undenormal(c.b2 * x - c.a2 * y);
        y
    }
}

/// Stereo biquad sharing one set of coefficients.
#[derive(Clone, Debug, Default)]
pub struct StereoBiquad {
    c: Coeffs,
    /// `[s1, s2]` per channel.
    s: [[f32; 2]; 2],
}

impl StereoBiquad {
    pub fn new(c: Coeffs) -> Self {
        Self { c, s: [[0.0; 2]; 2] }
    }

    /// Replaces the coefficients, keeping the state.
    pub fn set_coeffs(&mut self, c: Coeffs) {
        self.c = c;
    }

    pub fn coeffs(&self) -> Coeffs {
        self.c
    }

    pub fn reset(&mut self) {
        self.s = [[0.0; 2]; 2];
    }

    #[inline]
    pub fn tick(&mut self, x: [f32; 2]) -> [f32; 2] {
        let c = &self.c;
        let mut y = [0.0; 2];
        for ch in 0..2 {
            let s = &mut self.s[ch];
            y[ch] = c.b0 * x[ch] + s[0];
            s[0] = c.b1 * x[ch] - c.a1 * y[ch] + s[1];
            s[1] = c.b2 * x[ch] - c.a2 * y[ch];
        }
        y
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        for f in buf.iter_mut() {
            *f = self.tick(*f);
        }
        self.flush();
    }

    /// Flushes subnormal state; [`process`](Self::process) does this per block,
    /// callers of [`tick`](Self::tick) should do it once per block.
    pub fn flush(&mut self) {
        for s in self.s.iter_mut().flatten() {
            *s = undenormal(*s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::tone_gain_db;

    const SR: f32 = 48_000.0;

    fn measured(c: Coeffs, freq: f32) -> f32 {
        let mut f = StereoBiquad::new(c);
        tone_gain_db(SR, freq, |b| f.process(b))
    }

    #[test]
    fn cookbook_shapes() {
        let db = |c: Coeffs, f: f32| crate::gain_to_db(c.magnitude(SR, f));
        let q = std::f32::consts::FRAC_1_SQRT_2;
        assert!((db(Coeffs::lowpass(SR, 1000.0, q), 1000.0) + 3.01).abs() < 0.02);
        assert!(db(Coeffs::lowpass(SR, 1000.0, q), 100.0).abs() < 0.01);
        assert!((db(Coeffs::highpass(SR, 1000.0, q), 1000.0) + 3.01).abs() < 0.02);
        assert!(db(Coeffs::bandpass(SR, 1000.0, 2.0), 1000.0).abs() < 0.01);
        for f in [20.0, 1000.0, 15_000.0] {
            assert!(db(Coeffs::allpass(SR, 1000.0, q), f).abs() < 1e-4);
        }
        assert!((db(Coeffs::peaking(SR, 1000.0, 1.0, 6.0), 1000.0) - 6.0).abs() < 0.01);
        assert!((db(Coeffs::lowshelf(SR, 200.0, q, -9.0), 20.0) + 9.0).abs() < 0.1);
        assert!(db(Coeffs::lowshelf(SR, 200.0, q, -9.0), 10_000.0).abs() < 0.05);
        assert!((db(Coeffs::highshelf(SR, 5000.0, q, 4.0), 20_000.0) - 4.0).abs() < 0.1);
        assert!(db(Coeffs::highshelf(SR, 5000.0, q, 4.0), 50.0).abs() < 0.05);
    }

    #[test]
    fn filtering_matches_response() {
        let c = Coeffs::lowpass(SR, 2000.0, 0.9);
        for f in [100.0, 2000.0, 8000.0] {
            let want = crate::gain_to_db(c.magnitude(SR, f));
            assert!((measured(c, f) - want).abs() < 0.02, "{f}");
        }
        let mut m = Biquad::new(c);
        let mut s = StereoBiquad::new(c);
        for i in 0..100 {
            let x = (i as f32 * 0.37).sin();
            assert_eq!(m.tick(x), s.tick([x, 0.0])[0]);
        }
    }

    #[test]
    fn extreme_parameters_stay_finite() {
        for c in [
            Coeffs::lowpass(SR, 0.0, 0.0),
            Coeffs::highpass(SR, 1e9, 100.0),
            Coeffs::peaking(SR, f32::NAN, f32::NAN, 40.0),
        ] {
            let mut f = StereoBiquad::new(c);
            let mut buf = vec![[1.0f32, -1.0]; 4096];
            assert_no_alloc::assert_no_alloc(|| f.process(&mut buf));
            assert!(buf.iter().flatten().all(|v| v.is_finite()));
        }
    }
}
