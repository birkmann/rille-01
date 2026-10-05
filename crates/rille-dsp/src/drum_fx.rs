//! Drum machine effects: the per-track inserts (bit reduction, sample-rate
//! reduction, overdrive, multimode filter) and the bus compressor.
//!
//! Knobs take `0..=1`. Every insert has an "off" end where it is bypassed
//! and passes the signal through unchanged, so a fresh track costs nothing.

use crate::clamp01;
use crate::filter::Svf;
use crate::smooth::OnePole;
use crate::{db_to_gain, gain_to_db};

const PARAM_MS: f32 = 20.0;
/// Filter coefficients are recomputed every this many frames.
const SUB_BLOCK: usize = 16;

/// Bit reduction: quantises to `16 − 14 × knob` bits. Knob 0 = off.
#[derive(Clone, Debug, Default)]
pub struct BitCrush {
    knob: f32,
}

impl BitCrush {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_knob(&mut self, v: f32) {
        self.knob = clamp01(v);
    }

    pub fn is_bypassed(&self) -> bool {
        self.knob <= 0.0
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        if self.is_bypassed() {
            return;
        }
        let steps = 2f32.powf(15.0 - 14.0 * self.knob);
        let inv = 1.0 / steps;
        for f in buf {
            *f = [(f[0] * steps).round() * inv, (f[1] * steps).round() * inv];
        }
    }
}

/// Sample-rate reduction: holds each sample for 1 … 48 frames. Knob 0 = off.
#[derive(Clone, Debug)]
pub struct Downsample {
    knob: f32,
    phase: f32,
    held: [f32; 2],
}

impl Default for Downsample {
    fn default() -> Self {
        Self { knob: 0.0, phase: 1.0, held: [0.0; 2] }
    }
}

impl Downsample {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_knob(&mut self, v: f32) {
        self.knob = clamp01(v);
    }

    pub fn is_bypassed(&self) -> bool {
        self.knob <= 0.0
    }

    pub fn reset(&mut self) {
        self.phase = 1.0;
        self.held = [0.0; 2];
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        if self.is_bypassed() {
            // The first frame after switching on is taken at once.
            self.phase = 1.0;
            return;
        }
        let step = 1.0 / 48f32.powf(self.knob);
        for f in buf {
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                self.held = *f;
            }
            *f = self.held;
            self.phase += step;
        }
    }
}

/// Overdrive: a `tanh` shaper that keeps full scale at full scale, driven
/// up to +30 dB. Low settings blend it in, so turning it up from 0 is
/// smooth. Knob 0 = off.
#[derive(Clone, Debug)]
pub struct Drive {
    knob: f32,
    gain: OnePole,
    wet: OnePole,
}

impl Drive {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            knob: 0.0,
            gain: OnePole::new(sample_rate, PARAM_MS, 1.0),
            wet: OnePole::new(sample_rate, PARAM_MS, 0.0),
        }
    }

    pub fn set_knob(&mut self, v: f32) {
        self.knob = clamp01(v);
        self.gain.set_target(db_to_gain(self.knob * 30.0));
        self.wet.set_target((self.knob * 5.0).min(1.0));
    }

    pub fn is_bypassed(&self) -> bool {
        self.knob <= 0.0 && self.wet.is_settled()
    }

    pub fn reset(&mut self) {
        self.gain.set_immediate(self.gain.target());
        self.wet.set_immediate(self.wet.target());
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        if self.is_bypassed() {
            return;
        }
        for f in buf {
            let g = self.gain.tick();
            let w = self.wet.tick();
            let norm = 1.0 / g.tanh();
            for x in f.iter_mut() {
                *x += w * ((g * *x).tanh() * norm - *x);
            }
        }
    }
}

/// Filter types of [`MultiFilter`], picked by its type knob in three zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterType {
    LowPass,
    BandPass,
    HighPass,
}

impl FilterType {
    pub fn from_knob(v: f32) -> Self {
        match (clamp01(v) * 3.0) as usize {
            0 => Self::LowPass,
            1 => Self::BandPass,
            _ => Self::HighPass,
        }
    }
}

/// Resonant low-, band- or high-pass filter. Cutoff 20 Hz … 20 kHz,
/// resonance Q 0.7 … 10. Bypassed when it would do nothing: low-pass fully
/// open or high-pass fully closed, without resonance.
#[derive(Clone, Debug)]
pub struct MultiFilter {
    sample_rate: f32,
    svf: Svf,
    kind: FilterType,
    cutoff: OnePole,
    res: OnePole,
    /// `1 / Q` now (band-pass output gain, so its peak stays at unity).
    k: f32,
    left: usize,
    bypassed: bool,
}

impl MultiFilter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            svf: Svf::new(sample_rate, 20_000.0, std::f32::consts::FRAC_1_SQRT_2),
            kind: FilterType::LowPass,
            cutoff: OnePole::new(sample_rate / SUB_BLOCK as f32, PARAM_MS, 1.0),
            res: OnePole::new(sample_rate / SUB_BLOCK as f32, PARAM_MS, 0.0),
            k: std::f32::consts::SQRT_2,
            left: 0,
            bypassed: true,
        }
    }

    pub fn set(&mut self, cutoff: f32, res: f32, kind: f32) {
        self.cutoff.set_target(clamp01(cutoff));
        self.res.set_target(clamp01(res));
        self.kind = FilterType::from_knob(kind);
    }

    pub fn is_bypassed(&self) -> bool {
        let (c, r) = (self.cutoff.target(), self.res.target());
        let off = r <= 0.0
            && match self.kind {
                FilterType::LowPass => c >= 0.999,
                FilterType::BandPass => false,
                FilterType::HighPass => c <= 0.001,
            };
        off && self.cutoff.is_settled() && self.res.is_settled()
    }

    pub fn reset(&mut self) {
        self.cutoff.set_immediate(self.cutoff.target());
        self.res.set_immediate(self.res.target());
        self.svf.reset();
        self.left = 0;
    }

    fn update(&mut self) {
        self.left = SUB_BLOCK;
        let freq = 20.0 * 1000f32.powf(self.cutoff.tick());
        let q = std::f32::consts::FRAC_1_SQRT_2 * 14f32.powf(self.res.tick());
        self.k = 1.0 / q;
        self.svf.set(self.sample_rate, freq, q);
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        if self.is_bypassed() {
            self.bypassed = true;
            return;
        }
        if std::mem::take(&mut self.bypassed) {
            self.svf.reset();
            self.left = 0;
        }
        for f in buf.iter_mut() {
            if self.left == 0 {
                self.update();
            }
            self.left -= 1;
            let o = self.svf.tick(*f);
            *f = match self.kind {
                FilterType::LowPass => o.low,
                FilterType::BandPass => [o.band[0] * self.k, o.band[1] * self.k],
                FilterType::HighPass => o.high,
            };
        }
        self.svf.flush();
    }
}

/// One drum track's inserts in series: bit reduction, sample-rate
/// reduction, overdrive, filter.
#[derive(Clone, Debug)]
pub struct TrackFx {
    pub bits: BitCrush,
    pub srr: Downsample,
    pub drive: Drive,
    pub filter: MultiFilter,
}

impl TrackFx {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            bits: BitCrush::new(),
            srr: Downsample::new(),
            drive: Drive::new(sample_rate),
            filter: MultiFilter::new(sample_rate),
        }
    }

    pub fn is_bypassed(&self) -> bool {
        self.bits.is_bypassed() && self.srr.is_bypassed() && self.drive.is_bypassed() && self.filter.is_bypassed()
    }

    pub fn reset(&mut self) {
        self.srr.reset();
        self.drive.reset();
        self.filter.reset();
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        self.bits.process(buf);
        self.srr.process(buf);
        self.drive.process(buf);
        self.filter.process(buf);
    }
}

/// Soft knee width of the compressor, dB.
const KNEE_DB: f32 = 6.0;
const ATTACK_MS: f32 = 3.0;
/// Level the detector treats as silence, dB.
const FLOOR_DB: f32 = -120.0;

/// Feed-forward stereo compressor with a soft knee, automatic makeup gain,
/// a dry/wet mix and an optional external sidechain. Knobs: threshold
/// (−40 … 0 dB; 0 dB = off), ratio (1:1 … 20:1), release (30 ms … 1 s), mix.
#[derive(Clone, Debug)]
pub struct Compressor {
    sample_rate: f32,
    threshold_knob: f32,
    threshold: OnePole,
    ratio: OnePole,
    mix: OnePole,
    attack: f32,
    release: f32,
    /// Gain reduction now, dB (≤ 0).
    gr: f32,
}

impl Compressor {
    pub fn new(sample_rate: f32) -> Self {
        let mut c = Self {
            sample_rate,
            threshold_knob: 1.0,
            threshold: OnePole::new(sample_rate, PARAM_MS, 0.0),
            ratio: OnePole::new(sample_rate, PARAM_MS, 4.0),
            mix: OnePole::new(sample_rate, PARAM_MS, 1.0),
            attack: Self::coef(sample_rate, ATTACK_MS),
            release: 0.0,
            gr: 0.0,
        };
        c.set_release(0.4);
        c.reset();
        c
    }

    fn coef(sample_rate: f32, ms: f32) -> f32 {
        (-1.0 / (ms * 0.001 * sample_rate)).exp()
    }

    pub fn set_threshold(&mut self, v: f32) {
        self.threshold_knob = clamp01(v);
        self.threshold.set_target(-40.0 * (1.0 - self.threshold_knob));
    }

    pub fn set_ratio(&mut self, v: f32) {
        self.ratio.set_target(20f32.powf(clamp01(v)));
    }

    pub fn set_release(&mut self, v: f32) {
        let ms = 30.0 * (1000.0f32 / 30.0).powf(clamp01(v));
        self.release = Self::coef(self.sample_rate, ms);
    }

    pub fn set_mix(&mut self, v: f32) {
        self.mix.set_target(clamp01(v));
    }

    /// Gain reduction now, dB (≤ 0).
    pub fn gain_reduction_db(&self) -> f32 {
        self.gr
    }

    pub fn is_bypassed(&self) -> bool {
        self.threshold_knob >= 0.999 && self.threshold.is_settled() && self.gr > -0.01
    }

    pub fn reset(&mut self) {
        self.threshold.set_immediate(self.threshold.target());
        self.ratio.set_immediate(self.ratio.target());
        self.mix.set_immediate(self.mix.target());
        self.gr = 0.0;
    }

    /// Gain change in dB for a detector level of `x` dB.
    fn curve(x: f32, threshold: f32, ratio: f32) -> f32 {
        let over = x - threshold;
        let slope = 1.0 / ratio - 1.0;
        if 2.0 * over < -KNEE_DB {
            0.0
        } else if 2.0 * over.abs() <= KNEE_DB {
            slope * (over + KNEE_DB * 0.5).powi(2) / (2.0 * KNEE_DB)
        } else {
            slope * over
        }
    }

    /// Compresses `buf` in place, keyed by `sidechain` (same length) or by
    /// `buf` itself.
    pub fn process(&mut self, buf: &mut [[f32; 2]], sidechain: Option<&[[f32; 2]]>) {
        if self.is_bypassed() {
            self.gr = 0.0;
            return;
        }
        for (i, f) in buf.iter_mut().enumerate() {
            let key = sidechain.and_then(|s| s.get(i)).unwrap_or(f);
            let peak = key[0].abs().max(key[1].abs());
            let level = if peak > 1e-6 { gain_to_db(peak) } else { FLOOR_DB };
            let (t, r, m) = (self.threshold.tick(), self.ratio.tick(), self.mix.tick());
            let target = Self::curve(level, t, r);
            let a = if target < self.gr { self.attack } else { self.release };
            self.gr = target + a * (self.gr - target);
            let makeup = -t * (1.0 - 1.0 / r) * 0.5;
            let g = 1.0 + m * (db_to_gain(self.gr + makeup) - 1.0);
            *f = [f[0] * g, f[1] * g];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Rng, all_finite, sine};

    const SR: f32 = 48_000.0;

    fn noise(seed: u64, n: usize) -> Vec<[f32; 2]> {
        let mut rng = Rng::new(seed);
        (0..n).map(|_| [rng.bipolar(), rng.bipolar()]).collect()
    }

    #[test]
    fn off_settings_pass_the_signal_unchanged() {
        let input = noise(1, 4096);
        let mut fx = TrackFx::new(SR);
        assert!(fx.is_bypassed());
        let mut buf = input.clone();
        fx.process(&mut buf);
        assert_eq!(buf, input);
        let mut c = Compressor::new(SR);
        c.set_threshold(1.0);
        c.process(&mut buf, None);
        assert_eq!(buf, input);
    }

    #[test]
    fn high_pass_closed_is_off() {
        let mut f = MultiFilter::new(SR);
        f.set(0.0, 0.0, 1.0);
        f.reset();
        assert!(f.is_bypassed());
        f.set(0.0, 0.0, 0.5);
        assert!(!f.is_bypassed(), "band-pass always filters");
    }

    #[test]
    fn bit_crush_quantises() {
        let mut b = BitCrush::new();
        b.set_knob(1.0);
        let mut buf = vec![[0.3, -0.7]];
        b.process(&mut buf);
        // Two bits: steps of 0.5.
        assert_eq!(buf[0], [0.5, -0.5]);
    }

    #[test]
    fn downsample_holds_samples() {
        let mut d = Downsample::new();
        d.set_knob(1.0);
        let mut buf: Vec<[f32; 2]> = (0..96).map(|i| [i as f32, i as f32]).collect();
        d.process(&mut buf);
        assert!(buf[..47].iter().all(|f| f[0] == 0.0));
        assert!(matches!(buf[50][0], 48.0 | 49.0), "{}", buf[50][0]);
    }

    #[test]
    fn low_pass_cuts_highs() {
        let mut f = MultiFilter::new(SR);
        f.set(0.3, 0.0, 0.0);
        f.reset();
        let mut buf = sine(SR, 5000.0, 0.5, 9600);
        f.process(&mut buf);
        let peak = buf[4800..].iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!(peak < 0.05, "peak {peak}");
    }

    #[test]
    fn drive_keeps_full_scale_and_lifts_quiet_parts() {
        let mut d = Drive::new(SR);
        d.set_knob(1.0);
        d.reset();
        let mut buf = vec![[1.0, 0.1]; 16];
        d.process(&mut buf);
        assert!((buf[15][0] - 1.0).abs() < 1e-5);
        assert!(buf[15][1] > 0.5);
    }

    #[test]
    fn compressor_reduces_loud_signals() {
        let mut c = Compressor::new(SR);
        c.set_threshold(0.5); // −20 dB
        c.set_ratio(1.0); // 20:1
        c.set_mix(1.0);
        c.reset();
        let mut buf = vec![[0.9, 0.9]; 9600];
        c.process(&mut buf, None);
        assert!(c.gain_reduction_db() < -15.0, "gr {}", c.gain_reduction_db());
        assert!(buf[9599][0] < 0.9 * db_to_gain(-6.0));
    }

    #[test]
    fn sidechain_ducks_the_input() {
        let mut c = Compressor::new(SR);
        c.set_threshold(0.5);
        c.set_ratio(1.0);
        c.reset();
        let key = vec![[0.9, 0.9]; 4800];
        let mut buf = vec![[0.05, 0.05]; 4800];
        c.process(&mut buf, Some(&key));
        assert!(buf[4799][0] < 0.05 * db_to_gain(-6.0), "{}", buf[4799][0]);
        // Without the key the quiet input is left alone.
        let mut c = Compressor::new(SR);
        c.set_threshold(0.5);
        c.reset();
        let mut buf = vec![[0.05, 0.05]; 4800];
        c.process(&mut buf, None);
        assert!(buf[4799][0] > 0.05);
    }

    /// Random input up to +20 dB and random knob moves: output stays finite
    /// and nothing allocates.
    #[test]
    fn survive_random_automation() {
        let mut rng = Rng::new(7);
        let mut fx = TrackFx::new(SR);
        let mut comp = Compressor::new(SR);
        let mut buf = vec![[0.0f32; 2]; 1024];
        let key = noise(3, 1024);
        for _ in 0..600 {
            let n = 1 + rng.below(buf.len());
            let amp = if rng.below(10) == 0 { 10.0 } else { 1.0 };
            for f in &mut buf[..n] {
                *f = [amp * rng.bipolar(), amp * rng.bipolar()];
            }
            match rng.below(8) {
                0 => fx.bits.set_knob(rng.f32()),
                1 => fx.srr.set_knob(rng.f32()),
                2 => fx.drive.set_knob(rng.f32()),
                3 => fx.filter.set(rng.f32(), rng.f32(), rng.f32()),
                4 => comp.set_threshold(rng.f32()),
                5 => comp.set_ratio(rng.f32()),
                6 => comp.set_release(rng.f32()),
                _ => comp.set_mix(rng.f32()),
            }
            let sc = (rng.below(2) == 0).then(|| &key[..n]);
            assert_no_alloc::assert_no_alloc(|| {
                fx.process(&mut buf[..n]);
                comp.process(&mut buf[..n], sc);
            });
            assert!(all_finite(&buf[..n]));
            assert!(buf[..n].iter().flatten().all(|v| v.abs() < 1000.0), "blew up");
        }
    }
}
