//! Tempo-synced DJ effects and the FX unit that hosts them.
//!
//! Every effect preallocates its buffers in `new(sample_rate)` and processes
//! 100 % wet in place; the [`FxUnit`] does the dry/wet mix. Beat-synced
//! timing comes from the master clock through [`FxCtx`].

mod beatmasher;
mod delay;
mod flanger;
mod gater;
mod lfo_filter;
mod reverb;
mod unit;

pub use beatmasher::Beatmasher;
pub use delay::Delay;
pub use flanger::Flanger;
pub use gater::Gater;
pub use lfo_filter::LfoFilter;
pub use reverb::Reverb;
pub use unit::{DEFAULT_AMOUNT, FxUnit, SLOTS};

/// Context passed to effects each block, from the master clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxCtx {
    pub sample_rate: f32,
    /// Master tempo, > 0.
    pub bpm: f64,
    /// Master clock beat position at the first frame of the block. Continuous;
    /// increases by `bpm / 60 / sample_rate` per frame.
    pub beat_pos: f64,
    pub beats_per_bar: u32,
}

impl FxCtx {
    /// Tempo clamped to `1..=1000` (120 if not finite), so bad input cannot
    /// cause divisions by zero or absurd buffer positions.
    pub fn safe_bpm(&self) -> f64 {
        if self.bpm.is_finite() { self.bpm.clamp(1.0, 1000.0) } else { 120.0 }
    }

    /// Beats advanced per frame.
    pub fn beats_per_frame(&self) -> f64 {
        self.safe_bpm() / 60.0 / self.sample_rate as f64
    }

    /// Frames per beat.
    pub fn frames_per_beat(&self) -> f64 {
        60.0 * self.sample_rate as f64 / self.safe_bpm()
    }

    /// Beat position of frame `n` of the block.
    #[inline]
    pub fn beat_at(&self, n: usize) -> f64 {
        self.beat_pos + n as f64 * self.beats_per_frame()
    }

    /// The context for a sub-block starting `frames` frames later.
    pub fn advanced(&self, frames: usize) -> Self {
        Self { beat_pos: self.beat_at(frames), ..*self }
    }
}

/// A DJ effect. Parameter setters only store targets (smoothed while
/// processing); nothing allocates after construction.
pub trait Effect: Send {
    fn name(&self) -> &'static str;
    /// Names of the three knobs, "" for unused.
    fn knob_names(&self) -> [&'static str; 3];
    /// Names of the three buttons, "" for unused.
    fn button_names(&self) -> [&'static str; 3];
    /// Sets knob `idx` (`0..3`) to `v` in `0..=1`.
    fn set_knob(&mut self, idx: usize, v: f32);
    fn set_button(&mut self, idx: usize, on: bool);
    /// Clears the audio state (buffers, filters, envelopes) and jumps
    /// smoothed parameters to their targets. Parameters are kept.
    fn reset(&mut self);
    /// Processes one block 100 % wet, in place.
    fn process(&mut self, buf: &mut [[f32; 2]], ctx: &FxCtx);
    /// Whether the output keeps ringing after the input stops (delay,
    /// reverb). Effects without a tail are bypassed while the unit is off.
    fn has_tail(&self) -> bool {
        false
    }
    /// The unit was switched off: stop sustaining (e.g. drop freeze) so the
    /// tail can decay. The next button press re-enables it.
    fn release(&mut self) {}
}

/// Effect names by index, as used by [`FxUnit::select_effect`]; 0 is "None".
pub const EFFECT_NAMES: [&str; 7] = ["None", "Delay", "Reverb", "Filter", "Flanger", "Gater", "Beatmasher"];

/// Creates the effect with index `idx` of [`EFFECT_NAMES`] (`None` for 0 or
/// out of range).
pub fn new_effect(idx: usize, sample_rate: f32) -> Option<Box<dyn Effect>> {
    Some(match idx {
        1 => Box::new(Delay::new(sample_rate)),
        2 => Box::new(Reverb::new(sample_rate)),
        3 => Box::new(LfoFilter::new(sample_rate)),
        4 => Box::new(Flanger::new(sample_rate)),
        5 => Box::new(Gater::new(sample_rate)),
        6 => Box::new(Beatmasher::new(sample_rate)),
        _ => return None,
    })
}

/// Picks one of `steps` for a knob value, splitting `0..=1` into equal zones.
fn step<T: Copy>(steps: &[T], v: f32) -> T {
    let i = (crate::clamp01(v) * steps.len() as f32) as usize;
    steps[i.min(steps.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Rng, all_finite};

    #[test]
    fn ctx_math() {
        let c = FxCtx { sample_rate: 48_000.0, bpm: 120.0, beat_pos: 3.0, beats_per_bar: 4 };
        assert_eq!(c.frames_per_beat(), 24_000.0);
        assert_eq!(c.advanced(12_000).beat_pos, 3.5);
        let bad = FxCtx { bpm: f64::NAN, ..c };
        assert_eq!(bad.safe_bpm(), 120.0);
        assert_eq!(FxCtx { bpm: 0.0, ..c }.safe_bpm(), 1.0);
    }

    #[test]
    fn step_zones() {
        let s = [1, 2, 3, 4];
        assert_eq!(step(&s, 0.0), 1);
        assert_eq!(step(&s, 0.26), 2);
        assert_eq!(step(&s, 1.0), 4);
        assert_eq!(step(&s, f32::NAN), 1);
    }

    /// Random input (up to +20 dB), random knob/button automation, random
    /// block sizes and tempo jumps: output stays finite and nothing allocates.
    #[test]
    fn all_effects_survive_random_automation() {
        for (idx, &name) in EFFECT_NAMES.iter().enumerate().skip(1) {
            let mut fx = new_effect(idx, 48_000.0).unwrap();
            assert_eq!(fx.name(), name);
            let mut rng = Rng::new(idx as u64);
            let mut buf = vec![[0.0f32; 2]; 1024];
            let mut beat = 0.0;
            let mut bpm = 120.0;
            for block in 0..600 {
                let n = 1 + rng.below(buf.len());
                let amp = if rng.below(10) == 0 { 10.0 } else { 1.0 };
                for f in &mut buf[..n] {
                    *f = [amp * rng.bipolar(), amp * rng.bipolar()];
                }
                if rng.below(4) == 0 {
                    fx.set_knob(rng.below(3), rng.f32());
                }
                if rng.below(20) == 0 {
                    fx.set_button(rng.below(3), rng.below(2) == 0);
                }
                if rng.below(50) == 0 {
                    bpm = 60.0 + 140.0 * rng.f32() as f64;
                }
                if rng.below(100) == 0 {
                    beat += 7.3 * rng.bipolar() as f64; // seek / resync
                }
                if block == 300 {
                    fx.reset();
                }
                let ctx = FxCtx { sample_rate: 48_000.0, bpm, beat_pos: beat, beats_per_bar: 4 };
                assert_no_alloc::assert_no_alloc(|| fx.process(&mut buf[..n], &ctx));
                assert!(all_finite(&buf[..n]), "{} produced NaN/Inf", fx.name());
                assert!(buf[..n].iter().flatten().all(|v| v.abs() < 100.0), "{} blew up", fx.name());
                beat = ctx.beat_at(n);
            }
        }
    }
}
