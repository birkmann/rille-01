//! State-variable filter and the single-knob DJ filter.

use crate::clamp01;
use crate::denormal::undenormal;
use crate::smooth::OnePole;
use std::f32::consts::PI;

/// Stereo Cytomic (Andrew Simper) trapezoidal state-variable filter. One
/// `tick` gives low-, band- and high-pass outputs; coefficients can change
/// every frame without instability.
#[derive(Clone, Debug)]
pub struct Svf {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
}

/// Outputs of one [`Svf::tick`].
#[derive(Clone, Copy, Debug)]
pub struct SvfOut {
    pub low: [f32; 2],
    pub band: [f32; 2],
    pub high: [f32; 2],
}

impl Svf {
    pub fn new(sample_rate: f32, freq: f32, q: f32) -> Self {
        let mut s = Self { k: 1.0, a1: 0.0, a2: 0.0, a3: 0.0, ic1: [0.0; 2], ic2: [0.0; 2] };
        s.set(sample_rate, freq, q);
        s
    }

    /// Sets the resonance frequency (clamped to `10 Hz..0.49 * sample_rate`)
    /// and Q. Keeps the state.
    #[inline]
    pub fn set(&mut self, sample_rate: f32, freq: f32, q: f32) {
        let f = freq.max(10.0).min(0.49 * sample_rate);
        let g = (PI * f / sample_rate).tan();
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    pub fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
    }

    #[inline]
    #[allow(clippy::needless_range_loop)] // parallel per-channel arrays
    pub fn tick(&mut self, x: [f32; 2]) -> SvfOut {
        let mut o = SvfOut { low: [0.0; 2], band: [0.0; 2], high: [0.0; 2] };
        for ch in 0..2 {
            let v3 = x[ch] - self.ic2[ch];
            let v1 = self.a1 * self.ic1[ch] + self.a2 * v3;
            let v2 = self.ic2[ch] + self.a2 * self.ic1[ch] + self.a3 * v3;
            self.ic1[ch] = 2.0 * v1 - self.ic1[ch];
            self.ic2[ch] = 2.0 * v2 - self.ic2[ch];
            o.low[ch] = v2;
            o.band[ch] = v1;
            o.high[ch] = x[ch] - self.k * v1 - v2;
        }
        o
    }

    /// Flushes subnormal state; call once per block.
    pub fn flush(&mut self) {
        for s in self.ic1.iter_mut().chain(self.ic2.iter_mut()) {
            *s = undenormal(*s);
        }
    }
}

/// Which side of the DJ filter is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    LowPass,
    HighPass,
}

/// Knob distance from the centre that still counts as off.
const DEAD_ZONE: f32 = 0.03;
/// Travel beyond the dead zone over which the filtered signal is faded in.
const FADE_ZONE: f32 = 0.05;
const Q: f32 = 0.9;
const LP_MAX_HZ: f32 = 20_000.0;
const LP_MIN_HZ: f32 = 60.0;
const HP_MIN_HZ: f32 = 20.0;
const HP_MAX_HZ: f32 = 8000.0;
/// Coefficients are recomputed every this many frames.
const SUB_BLOCK: u32 = 16;
const KNOB_SMOOTH_MS: f32 = 25.0;

/// −3 dB frequency of a 2nd-order low-pass relative to its resonance
/// frequency, for Q = 0.9 (the high-pass mirrors it).
fn cutoff_ratio() -> f32 {
    let b = 2.0 - 1.0 / (Q * Q);
    ((b + (b * b + 4.0).sqrt()) / 2.0).sqrt()
}

/// Single-knob DJ filter: 0.5 is off, turning left sweeps a low-pass from
/// 20 kHz down to 60 Hz, turning right a high-pass from 20 Hz up to 8 kHz
/// (both exponential). The filtered signal fades in over the first part of
/// the travel so engaging never clicks; in the centre the output is the
/// input, bit for bit.
#[derive(Clone, Debug)]
pub struct DjFilter {
    sample_rate: f32,
    svf: Svf,
    /// Smoothed knob position, advanced once per sub-block.
    knob: OnePole,
    mode: FilterMode,
    wet: f32,
    wet_target: f32,
    wet_step: f32,
    /// Frames left in the current sub-block.
    left: u32,
    active: bool,
}

impl DjFilter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            svf: Svf::new(sample_rate, 1000.0, Q),
            knob: OnePole::new(sample_rate / SUB_BLOCK as f32, KNOB_SMOOTH_MS, 0.5),
            mode: FilterMode::LowPass,
            wet: 0.0,
            wet_target: 0.0,
            wet_step: 0.0,
            left: 0,
            active: false,
        }
    }

    /// Sets the knob (`0..=1`, 0.5 = off).
    pub fn set_knob(&mut self, v: f32) {
        self.knob.set_target(clamp01(v));
    }

    pub fn knob(&self) -> f32 {
        self.knob.target()
    }

    /// True while the output is the untouched input.
    pub fn is_bypassed(&self) -> bool {
        !self.active
    }

    /// Mode and −3 dB cutoff for a knob position, `None` in the centre.
    pub fn cutoff_for_knob(knob: f32) -> Option<(FilterMode, f32)> {
        let d = clamp01(knob) - 0.5;
        if d.abs() <= DEAD_ZONE {
            return None;
        }
        let t = (d.abs() - DEAD_ZONE) / (0.5 - DEAD_ZONE);
        Some(if d < 0.0 {
            (FilterMode::LowPass, LP_MAX_HZ * (LP_MIN_HZ / LP_MAX_HZ).powf(t))
        } else {
            (FilterMode::HighPass, HP_MIN_HZ * (HP_MAX_HZ / HP_MIN_HZ).powf(t))
        })
    }

    /// Clears the state and jumps the knob to its target.
    pub fn reset(&mut self) {
        self.knob.set_immediate(self.knob.target());
        self.svf.reset();
        self.wet = 0.0;
        self.wet_target = 0.0;
        self.wet_step = 0.0;
        self.left = 0;
        self.active = false;
        let k = self.knob.value();
        if Self::cutoff_for_knob(k).is_some() {
            // Start fully engaged instead of fading in from dry.
            self.update();
            self.wet = self.wet_target;
            self.wet_step = 0.0;
        }
    }

    fn wet_for_knob(knob: f32) -> f32 {
        ((knob - 0.5).abs() - DEAD_ZONE).clamp(0.0, FADE_ZONE) / FADE_ZONE
    }

    /// Starts a sub-block: advances the knob and updates cutoff and wet ramp.
    fn update(&mut self) {
        self.left = SUB_BLOCK;
        let k = self.knob.tick();
        let mut wet = Self::wet_for_knob(k);
        match Self::cutoff_for_knob(k) {
            Some((mode, _)) if mode != self.mode && self.wet > 0.0 => wet = 0.0, // fade out before switching
            Some((mode, f)) => {
                self.mode = mode;
                let r = cutoff_ratio();
                let f0 = if mode == FilterMode::LowPass { f / r } else { f * r };
                self.svf.set(self.sample_rate, f0, Q);
            }
            None => {}
        }
        self.wet_target = wet;
        self.wet_step = (wet - self.wet) / SUB_BLOCK as f32;
        if wet == 0.0 && self.wet == 0.0 {
            if self.active {
                self.svf.reset();
            }
            self.active = false;
        } else {
            self.active = true;
        }
    }

    #[inline]
    pub fn tick(&mut self, x: [f32; 2]) -> [f32; 2] {
        if self.left == 0 {
            self.update();
        }
        self.left -= 1;
        if !self.active {
            return x;
        }
        let w = self.wet;
        // Land exactly on the target so the next sub-block can detect "fully dry".
        self.wet = if self.left == 0 { self.wet_target } else { self.wet + self.wet_step };
        let o = self.svf.tick(x);
        let y = if self.mode == FilterMode::LowPass { o.low } else { o.high };
        [x[0] + w * (y[0] - x[0]), x[1] + w * (y[1] - x[1])]
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        for f in buf.iter_mut() {
            *f = self.tick(*f);
        }
        self.svf.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Rng, all_finite, tone_gain_db};

    const SR: f32 = 48_000.0;

    #[test]
    fn centre_is_bit_transparent() {
        let mut f = DjFilter::new(SR);
        let mut rng = Rng::new(1);
        let input: Vec<[f32; 2]> = (0..10_000).map(|_| [rng.bipolar(), rng.bipolar()]).collect();
        let mut buf = input.clone();
        f.set_knob(0.52);
        f.process(&mut buf);
        assert_eq!(buf, input);
        // Sweep away and back: once settled in the centre, transparent again.
        f.set_knob(0.1);
        f.process(&mut buf);
        f.set_knob(0.5);
        let mut buf = input.clone();
        f.process(&mut buf);
        assert!(f.is_bypassed());
        let mut buf = input.clone();
        f.process(&mut buf);
        assert_eq!(buf, input);
    }

    #[test]
    fn minus_3db_at_cutoff() {
        for knob in [0.25, 0.3, 0.75, 0.62] {
            let (mode, fc) = DjFilter::cutoff_for_knob(knob).unwrap();
            let mut f = DjFilter::new(SR);
            f.set_knob(knob);
            f.reset();
            let g = tone_gain_db(SR, fc.round(), |b| f.process(b));
            assert!((g + 3.0).abs() < 1.0, "{mode:?} {fc} Hz: {g} dB");
            // An octave (LP: above, HP: below) is well down.
            let far = if mode == FilterMode::LowPass { fc * 4.0 } else { fc / 4.0 };
            let mut f2 = DjFilter::new(SR);
            f2.set_knob(knob);
            f2.reset();
            let g = tone_gain_db(SR, far.round(), |b| f2.process(b));
            assert!(g < -18.0, "{mode:?} {far} Hz: {g} dB");
        }
    }

    #[test]
    fn sweeps_are_click_free_and_finite() {
        let mut f = DjFilter::new(SR);
        let mut rng = Rng::new(9);
        let mut buf = vec![[0.0f32; 2]; 128];
        let mut prev = 0.0f32;
        let mut phase = 0.0f32;
        for block in 0..2000 {
            if block % 25 == 0 {
                f.set_knob(rng.f32());
            }
            for s in buf.iter_mut() {
                phase += 2.0 * PI * 80.0 / SR;
                let v = phase.sin() * 0.5;
                *s = [v, v];
            }
            assert_no_alloc::assert_no_alloc(|| f.process(&mut buf));
            assert!(all_finite(&buf));
            for s in &buf {
                assert!((s[0] - prev).abs() < 0.05, "jump {prev} -> {}", s[0]);
                prev = s[0];
            }
        }
    }
}
