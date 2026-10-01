//! Real-time-safe DSP building blocks and the DJ effects.
//!
//! Audio is processed in blocks of stereo frames (`[[f32; 2]]`). Constructors
//! allocate everything up front; `process*`, `tick` and `set_*` methods never
//! allocate, lock or panic on normal input, and parameter changes are smoothed.

pub mod biquad;
pub mod delay_line;
pub mod denormal;
pub mod eq;
pub mod filter;
pub mod fx;
pub mod interp;
pub mod limiter;
pub mod meter;
pub mod smooth;

pub use biquad::{Biquad, Coeffs, StereoBiquad};
pub use delay_line::DelayLine;
pub use denormal::flush_denormals;
pub use eq::IsolatorEq;
pub use filter::{DjFilter, FilterMode, Svf};
pub use fx::{Beatmasher, Delay, Effect, Flanger, FxCtx, FxUnit, Gater, LfoFilter, Reverb};
pub use interp::{SincTable, cubic_hermite};
pub use limiter::{PeakLimiter, SoftClip};
pub use meter::PeakMeter;
pub use smooth::{LinearSmoother, OnePole};

/// One stereo frame, `[left, right]`.
pub type Frame = [f32; 2];

/// Clamps a knob value to `0..=1`; NaN maps to 0.
#[inline]
pub(crate) fn clamp01(v: f32) -> f32 {
    if v >= 0.0 { v.min(1.0) } else { 0.0 }
}

/// Converts decibels to a linear gain.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Converts a linear gain to decibels (−inf for 0).
#[inline]
pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.log10()
}

#[cfg(test)]
mod test_util;

// assert_no_alloc only checks in debug builds by default (`disable_release`).
#[cfg(all(test, debug_assertions))]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
