//! Helpers shared by the unit tests.

use crate::fx::FxCtx;
use std::f64::consts::TAU;

/// Small deterministic xorshift generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform in `[0, 1)`.
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[-1, 1)`.
    pub fn bipolar(&mut self) -> f32 {
        self.f32() * 2.0 - 1.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Stereo sine, both channels equal.
pub fn sine(sample_rate: f32, freq: f64, amp: f32, frames: usize) -> Vec<[f32; 2]> {
    (0..frames)
        .map(|i| {
            let v = amp * (TAU * freq * i as f64 / sample_rate as f64).sin() as f32;
            [v, v]
        })
        .collect()
}

/// Amplitude of the `freq` component of `buf[..]` (left channel), by
/// correlation. Exact when the buffer holds whole cycles.
pub fn tone_amplitude(sample_rate: f32, freq: f64, buf: &[[f32; 2]], start_frame: usize) -> f64 {
    let (mut s, mut c) = (0.0, 0.0);
    for (i, f) in buf.iter().enumerate() {
        let ph = TAU * freq * (start_frame + i) as f64 / sample_rate as f64;
        s += f[0] as f64 * ph.sin();
        c += f[0] as f64 * ph.cos();
    }
    2.0 * s.hypot(c) / buf.len() as f64
}

/// Steady-state gain in dB of `process` for a sine at an integer `freq`:
/// one second to settle, one second measured.
pub fn tone_gain_db(sample_rate: f32, freq: f32, mut process: impl FnMut(&mut [[f32; 2]])) -> f32 {
    let sr = sample_rate as usize;
    let mut buf = sine(sample_rate, freq as f64, 0.5, 2 * sr);
    for block in buf.chunks_mut(256) {
        process(block);
    }
    let amp = tone_amplitude(sample_rate, freq as f64, &buf[sr..], sr);
    (20.0 * (amp / 0.5).log10()) as f32
}

pub fn ctx(bpm: f64, beat_pos: f64) -> FxCtx {
    FxCtx { sample_rate: 48_000.0, bpm, beat_pos, beats_per_bar: 4 }
}

pub fn all_finite(buf: &[[f32; 2]]) -> bool {
    buf.iter().flatten().all(|v| v.is_finite())
}
