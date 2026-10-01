//! Resonant filter swept by a beat-synced LFO.

use super::{Effect, FxCtx, step};
use crate::clamp01;
use crate::filter::Svf;
use crate::smooth::OnePole;
use std::f64::consts::TAU;

/// LFO periods in beats.
const RATES: [f64; 8] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
const Q: f32 = 1.3;
const MIN_HZ: f32 = 60.0;
const MAX_HZ: f32 = 16_000.0;
/// Modulation range at full depth, ± octaves.
const MAX_OCTAVES: f32 = 3.0;
const SUB_BLOCK: u32 = 16;
const PARAM_MS: f32 = 30.0;
/// Short smoothing of the modulation, so rate changes (phase jumps) glide.
const MOD_MS: f32 = 3.0;

/// Low- or high-pass filter whose cutoff follows a sine LFO locked to the
/// master beat. Knobs: cutoff, rate (1/4 … 32 beats), depth. Button: HP
/// (high-pass instead of low-pass, crossfaded).
pub struct LfoFilter {
    sample_rate: f32,
    svf: Svf,
    period: f64,
    /// log2 of the centre frequency.
    cutoff: OnePole,
    depth: OnePole,
    modulation: OnePole,
    hp: OnePole,
    left: u32,
}

impl LfoFilter {
    pub fn new(sample_rate: f32) -> Self {
        let block_rate = sample_rate / SUB_BLOCK as f32;
        let mut f = Self {
            sample_rate,
            svf: Svf::new(sample_rate, 1000.0, Q),
            period: 1.0,
            cutoff: OnePole::new(block_rate, PARAM_MS, 0.0),
            depth: OnePole::new(block_rate, PARAM_MS, 0.0),
            modulation: OnePole::new(block_rate, MOD_MS, 0.0),
            hp: OnePole::new(sample_rate, 10.0, 0.0),
            left: 0,
        };
        f.set_knob(0, 0.5);
        f.set_knob(1, 0.4);
        f.set_knob(2, 0.5);
        f.reset();
        f
    }

    fn update(&mut self, beat: f64) {
        self.left = SUB_BLOCK;
        let lfo = (TAU * (beat / self.period).rem_euclid(1.0)).sin() as f32;
        let oct = self.modulation.filter(lfo * self.depth.tick() * MAX_OCTAVES);
        self.svf.set(self.sample_rate, (self.cutoff.tick() + oct).exp2(), Q);
    }
}

impl Effect for LfoFilter {
    fn name(&self) -> &'static str {
        "Filter"
    }

    fn knob_names(&self) -> [&'static str; 3] {
        ["cutoff", "rate", "depth"]
    }

    fn button_names(&self) -> [&'static str; 3] {
        ["HP", "", ""]
    }

    fn set_knob(&mut self, idx: usize, v: f32) {
        match idx {
            0 => self.cutoff.set_target(MIN_HZ.log2() + clamp01(v) * (MAX_HZ / MIN_HZ).log2()),
            1 => self.period = step(&RATES, v),
            2 => self.depth.set_target(clamp01(v)),
            _ => {}
        }
    }

    fn set_button(&mut self, idx: usize, on: bool) {
        if idx == 0 {
            self.hp.set_target(if on { 1.0 } else { 0.0 });
        }
    }

    fn reset(&mut self) {
        self.svf.reset();
        for p in [&mut self.cutoff, &mut self.depth, &mut self.hp] {
            p.set_immediate(p.target());
        }
        self.modulation.set_immediate(0.0);
        self.left = 0;
    }

    fn process(&mut self, buf: &mut [[f32; 2]], ctx: &FxCtx) {
        for (n, f) in buf.iter_mut().enumerate() {
            if self.left == 0 {
                self.update(ctx.beat_at(n));
            }
            self.left -= 1;
            let o = self.svf.tick(*f);
            let m = self.hp.tick();
            *f = [o.low[0] + m * (o.high[0] - o.low[0]), o.low[1] + m * (o.high[1] - o.low[1])];
        }
        self.svf.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{ctx, sine};

    /// Output level of a high tone over one LFO cycle: loud where the LFO
    /// opens the low-pass (a quarter period in), quiet at three quarters.
    #[test]
    fn lfo_follows_beat_phase() {
        let mut f = LfoFilter::new(48_000.0);
        f.set_knob(0, 0.5);
        f.set_knob(1, 0.3); // 1 beat
        f.set_knob(2, 1.0);
        f.reset();
        let mut buf = sine(48_000.0, 4000.0, 0.5, 48_000);
        for (i, b) in buf.chunks_mut(64).enumerate() {
            f.process(b, &ctx(120.0, i as f64 * 64.0 / 24_000.0));
        }
        let level = |beat: f64| {
            let c = (beat * 24_000.0) as usize;
            buf[c - 600..c + 600].iter().fold(0f32, |m, v| m.max(v[0].abs()))
        };
        assert!(level(1.25) > 0.4, "{}", level(1.25));
        assert!(level(1.75) < 0.02, "{}", level(1.75));
        assert!(level(0.75) < 0.02 && level(0.25) > 0.4);
    }
}
