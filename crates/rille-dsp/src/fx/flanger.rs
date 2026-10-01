//! Beat-synced flanger.

use super::{Effect, FxCtx, step};
use crate::clamp01;
use crate::delay_line::DelayLine;
use crate::denormal::undenormal;
use crate::limiter::SoftClip;
use crate::smooth::OnePole;
use std::f64::consts::TAU;

/// LFO periods in beats.
const PERIODS: [f64; 6] = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
const MIN_MS: f32 = 0.5;
const MAX_MS: f32 = 8.0;
const MAX_FEEDBACK: f32 = 0.9;
const PARAM_MS: f32 = 20.0;
/// Smoothing of the delay time, so period changes (phase jumps) glide.
const DELAY_MS: f32 = 10.0;

/// Flanger: 0.5–8 ms delay swept by a raised-cosine LFO locked to the master
/// beat (right channel a quarter cycle ahead). Knobs: period (1 … 32 beats),
/// feedback, depth (sweep range).
pub struct Flanger {
    sample_rate: f32,
    line: DelayLine,
    period: f64,
    feedback: OnePole,
    depth: OnePole,
    delay: [OnePole; 2],
}

impl Flanger {
    pub fn new(sample_rate: f32) -> Self {
        let min = MIN_MS * 0.001 * sample_rate;
        let mut f = Self {
            sample_rate,
            line: DelayLine::new((MAX_MS * 0.001 * sample_rate) as usize + 4),
            period: 4.0,
            feedback: OnePole::new(sample_rate, PARAM_MS, 0.0),
            depth: OnePole::new(sample_rate, PARAM_MS, 0.0),
            delay: [OnePole::new(sample_rate, DELAY_MS, min), OnePole::new(sample_rate, DELAY_MS, min)],
        };
        f.set_knob(0, 0.4);
        f.set_knob(1, 0.5);
        f.set_knob(2, 1.0);
        f.reset();
        f
    }
}

impl Effect for Flanger {
    fn name(&self) -> &'static str {
        "Flanger"
    }

    fn knob_names(&self) -> [&'static str; 3] {
        ["period", "feedback", "depth"]
    }

    fn button_names(&self) -> [&'static str; 3] {
        ["", "", ""]
    }

    fn set_knob(&mut self, idx: usize, v: f32) {
        match idx {
            0 => self.period = step(&PERIODS, v),
            1 => self.feedback.set_target(clamp01(v) * MAX_FEEDBACK),
            2 => self.depth.set_target(clamp01(v)),
            _ => {}
        }
    }

    fn set_button(&mut self, _idx: usize, _on: bool) {}

    fn reset(&mut self) {
        self.line.clear();
        for p in [&mut self.feedback, &mut self.depth] {
            p.set_immediate(p.target());
        }
        let min = MIN_MS * 0.001 * self.sample_rate;
        self.delay.iter_mut().for_each(|d| d.set_immediate(min));
    }

    fn process(&mut self, buf: &mut [[f32; 2]], ctx: &FxCtx) {
        let ms_to_frames = 0.001 * self.sample_rate;
        for (n, f) in buf.iter_mut().enumerate() {
            let phase = (ctx.beat_at(n) / self.period).rem_euclid(1.0);
            let depth = self.depth.tick();
            let d: [f32; 2] = std::array::from_fn(|ch| {
                let lfo = 0.5 - 0.5 * (TAU * (phase + 0.25 * ch as f64)).cos() as f32;
                self.delay[ch].filter((MIN_MS + (MAX_MS - MIN_MS) * depth * lfo) * ms_to_frames)
            });
            let y = [self.line.read(d[0])[0], self.line.read(d[1])[1]];
            let fb = self.feedback.tick();
            let w = |ch: usize| undenormal(SoftClip::shape(f[ch] + fb * y[ch], 1.0, 2.0));
            self.line.write([w(0), w(1)]);
            *f = [0.5 * (f[0] + y[0]), 0.5 * (f[1] + y[1])];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{ctx, sine};

    /// A tone is notched when the delay puts it in anti-phase and passes
    /// when in phase; the sweep must cover both over one LFO period.
    #[test]
    fn sweeps_comb_over_period() {
        let mut f = Flanger::new(48_000.0);
        f.set_knob(0, 0.0); // 1 beat
        f.set_knob(1, 0.0);
        f.set_knob(2, 1.0);
        f.reset();
        let mut buf = sine(48_000.0, 1000.0, 0.5, 48_000);
        for (i, b) in buf.chunks_mut(128).enumerate() {
            f.process(b, &ctx(120.0, i as f64 * 128.0 / 24_000.0));
        }
        let env: Vec<f32> =
            buf[24_000..48_000].chunks(480).map(|c| c.iter().fold(0f32, |m, v| m.max(v[0].abs()))).collect();
        let (lo, hi) = env.iter().fold((1f32, 0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(hi > 0.45 && lo < 0.1, "{lo} {hi}");
    }
}
