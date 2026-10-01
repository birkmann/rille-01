//! Three-band DJ isolator EQ.
//!
//! Linkwitz-Riley 4th-order crossovers split the signal into low, mid and
//! high bands. An LR4 low-pass plus high-pass sums to a 2nd-order all-pass, so
//! the low band gets the same all-pass as the mid/high split and the three
//! bands sum flat at unity gain (only phase changes).

use crate::biquad::{Coeffs, StereoBiquad};
use crate::clamp01;
use crate::smooth::LinearSmoother;
use std::f32::consts::FRAC_1_SQRT_2;

/// Low/mid crossover frequency.
pub const LOW_MID_HZ: f32 = 200.0;
/// Mid/high crossover frequency.
pub const MID_HIGH_HZ: f32 = 2500.0;
/// Maximum boost at knob 1.0.
pub const MAX_BOOST_DB: f32 = 6.0;

const GAIN_RAMP_MS: f32 = 20.0;

/// Maps an EQ knob (`0..=1`) to a linear band gain: 0.5 is 0 dB, 1.0 is
/// +6 dB (linear in dB), and below 0.5 the gain falls as `(k / 0.5)^2` to a
/// full kill at 0.
pub fn knob_to_gain(knob: f32) -> f32 {
    let k = clamp01(knob);
    if k >= 0.5 { crate::db_to_gain((k - 0.5) * 2.0 * MAX_BOOST_DB) } else { (k * 2.0) * (k * 2.0) }
}

/// Linkwitz-Riley 4th order = two identical Butterworth biquads.
#[derive(Clone, Debug)]
struct Lr4([StereoBiquad; 2]);

impl Lr4 {
    fn new(c: Coeffs) -> Self {
        Self([StereoBiquad::new(c), StereoBiquad::new(c)])
    }

    #[inline]
    fn tick(&mut self, x: [f32; 2]) -> [f32; 2] {
        let y = self.0[0].tick(x);
        self.0[1].tick(y)
    }

    fn reset(&mut self) {
        self.0.iter_mut().for_each(StereoBiquad::reset);
    }

    fn flush(&mut self) {
        self.0.iter_mut().for_each(StereoBiquad::flush);
    }
}

/// Three-band isolator with smoothed band gains.
#[derive(Clone, Debug)]
pub struct IsolatorEq {
    lp1: Lr4,
    hp1: Lr4,
    lp2: Lr4,
    hp2: Lr4,
    /// Compensates the low band for the phase of the mid/high split.
    ap2: StereoBiquad,
    gains: [LinearSmoother; 3],
}

impl IsolatorEq {
    pub fn new(sample_rate: f32) -> Self {
        let q = FRAC_1_SQRT_2;
        let sr = sample_rate;
        Self {
            lp1: Lr4::new(Coeffs::lowpass(sr, LOW_MID_HZ, q)),
            hp1: Lr4::new(Coeffs::highpass(sr, LOW_MID_HZ, q)),
            lp2: Lr4::new(Coeffs::lowpass(sr, MID_HIGH_HZ, q)),
            hp2: Lr4::new(Coeffs::highpass(sr, MID_HIGH_HZ, q)),
            ap2: StereoBiquad::new(Coeffs::allpass(sr, MID_HIGH_HZ, q)),
            gains: std::array::from_fn(|_| LinearSmoother::new(sr, GAIN_RAMP_MS, 1.0)),
        }
    }

    /// Sets the three knobs (`0..=1`, 0.5 = flat); see [`knob_to_gain`].
    pub fn set_knobs(&mut self, lo: f32, mid: f32, hi: f32) {
        for (g, k) in self.gains.iter_mut().zip([lo, mid, hi]) {
            g.set_target(knob_to_gain(k));
        }
    }

    /// Clears the filter state and jumps the gains to their targets.
    pub fn reset(&mut self) {
        for f in [&mut self.lp1, &mut self.hp1, &mut self.lp2, &mut self.hp2] {
            f.reset();
        }
        self.ap2.reset();
        for g in &mut self.gains {
            g.set_immediate(g.target());
        }
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        for f in buf.iter_mut() {
            let [gl, gm, gh] = [self.gains[0].tick(), self.gains[1].tick(), self.gains[2].tick()];
            let lo = self.ap2.tick(self.lp1.tick(*f));
            let rest = self.hp1.tick(*f);
            let mid = self.lp2.tick(rest);
            let hi = self.hp2.tick(rest);
            for ch in 0..2 {
                f[ch] = gl * lo[ch] + gm * mid[ch] + gh * hi[ch];
            }
        }
        for f in [&mut self.lp1, &mut self.hp1, &mut self.lp2, &mut self.hp2] {
            f.flush();
        }
        self.ap2.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Rng, all_finite, tone_gain_db};

    const SR: f32 = 48_000.0;

    fn gain_db(knobs: [f32; 3], freq: f32) -> f32 {
        let mut eq = IsolatorEq::new(SR);
        eq.set_knobs(knobs[0], knobs[1], knobs[2]);
        eq.reset();
        tone_gain_db(SR, freq, |b| eq.process(b))
    }

    #[test]
    fn knob_curve() {
        assert_eq!(knob_to_gain(0.5), 1.0);
        assert!((crate::gain_to_db(knob_to_gain(1.0)) - 6.0).abs() < 1e-4);
        assert!((crate::gain_to_db(knob_to_gain(0.75)) - 3.0).abs() < 1e-4);
        assert_eq!(knob_to_gain(0.25), 0.25);
        assert_eq!(knob_to_gain(0.0), 0.0);
        assert_eq!(knob_to_gain(f32::NAN), 0.0);
    }

    #[test]
    fn unity_sums_flat() {
        for f in [20, 30, 50, 100, 150, 200, 300, 500, 1000, 2000, 2500, 3000, 5000, 8000, 10_000, 15_000, 20_000] {
            let g = gain_db([0.5; 3], f as f32);
            assert!(g.abs() < 0.05, "{f} Hz: {g} dB");
        }
    }

    #[test]
    fn kills_are_deep_in_band() {
        // LR4 slopes are 24 dB/octave: 50 Hz (two octaves below the
        // crossover) gets ~48 dB, so the low kill is measured at 40 Hz. The
        // high side is steeper thanks to the bilinear transform (53 dB at 10 kHz).
        let lo = gain_db([0.0, 0.5, 0.5], 40.0);
        let hi = gain_db([0.5, 0.5, 0.0], 10_000.0);
        let mid = gain_db([0.5, 0.0, 0.5], 707.0);
        assert!(lo < -50.0, "lo kill {lo}");
        assert!(hi < -50.0, "hi kill {hi}");
        assert!(mid < -20.0, "mid kill {mid}"); // mid band is only 3.6 octaves wide
        assert!(gain_db([0.5, 0.5, 0.5], 40.0).abs() < 0.05);
        let boost = gain_db([1.0, 0.5, 0.5], 40.0);
        assert!((boost - 6.0).abs() < 0.1, "{boost}");
    }

    #[test]
    fn gain_changes_are_smooth_and_finite() {
        let mut eq = IsolatorEq::new(SR);
        let mut rng = Rng::new(3);
        let mut buf = vec![[0.0f32; 2]; 256];
        let mut prev = 0.0f32;
        for block in 0..400 {
            if block % 10 == 0 {
                eq.set_knobs(rng.f32(), rng.f32(), rng.f32());
            }
            // Low-frequency sine: any gain step would show up as a jump.
            for (i, f) in buf.iter_mut().enumerate() {
                let v = (((block * 256 + i) as f32) * 2.0 * std::f32::consts::PI * 60.0 / SR).sin() * 0.5;
                *f = [v, v];
            }
            assert_no_alloc::assert_no_alloc(|| eq.process(&mut buf));
            assert!(all_finite(&buf));
            for f in &buf {
                assert!((f[0] - prev).abs() < 0.02, "jump {prev} -> {}", f[0]);
                prev = f[0];
            }
        }
    }
}
