//! Beat-synced amplitude gate.

use super::{Effect, FxCtx, step};
use crate::clamp01;
use crate::smooth::OnePole;
use std::f32::consts::PI;

/// Gate periods in beats.
const RATES: [f64; 4] = [0.25, 0.125, 0.0625, 0.03125];
/// Share of each period the gate is open (before the release).
const OPEN: f32 = 0.5;
/// Shortest attack/release, so hard gating does not click.
const MIN_RAMP_SECS: f64 = 0.0005;
const PARAM_MS: f32 = 20.0;

/// Gain of the gate at `phase` in the period: raised-cosine attack of
/// length `a`, open until [`OPEN`], raised-cosine release of length `r`.
fn gate(phase: f32, a: f32, r: f32) -> f32 {
    if phase < a {
        0.5 - 0.5 * (PI * phase / a).cos()
    } else if phase < OPEN {
        1.0
    } else if phase < OPEN + r {
        0.5 + 0.5 * (PI * (phase - OPEN) / r).cos()
    } else {
        0.0
    }
}

/// Rhythmic gate. Each period starts opening exactly at a multiple of the
/// rate in `beat_pos`; the envelope is computed from the beat phase, not
/// from a running state, so it cannot drift. Knobs: rate (1/4 … 1/32 beat),
/// shape (attack/release smoothness), depth. Rate changes wait for the next
/// quarter beat, where every rate starts a period, so they never click.
pub struct Gater {
    rate: f64,
    pending: f64,
    shape: OnePole,
    depth: OnePole,
    last_beat: f64,
}

impl Gater {
    pub fn new(sample_rate: f32) -> Self {
        let mut g = Self {
            rate: RATES[1],
            pending: RATES[1],
            shape: OnePole::new(sample_rate, PARAM_MS, 0.3),
            depth: OnePole::new(sample_rate, PARAM_MS, 1.0),
            last_beat: f64::NAN,
        };
        g.reset();
        g
    }
}

impl Effect for Gater {
    fn name(&self) -> &'static str {
        "Gater"
    }

    fn knob_names(&self) -> [&'static str; 3] {
        ["rate", "shape", "depth"]
    }

    fn button_names(&self) -> [&'static str; 3] {
        ["", "", ""]
    }

    fn set_knob(&mut self, idx: usize, v: f32) {
        match idx {
            0 => self.pending = step(&RATES, v),
            1 => self.shape.set_target(clamp01(v)),
            2 => self.depth.set_target(clamp01(v)),
            _ => {}
        }
    }

    fn set_button(&mut self, _idx: usize, _on: bool) {}

    fn reset(&mut self) {
        self.shape.set_immediate(self.shape.target());
        self.depth.set_immediate(self.depth.target());
        self.last_beat = f64::NAN;
    }

    fn process(&mut self, buf: &mut [[f32; 2]], ctx: &FxCtx) {
        let spb = 60.0 / ctx.safe_bpm();
        for (n, f) in buf.iter_mut().enumerate() {
            let beat = ctx.beat_at(n);
            if self.pending != self.rate && (beat * 4.0).floor() != (self.last_beat * 4.0).floor() {
                self.rate = self.pending;
            }
            self.last_beat = beat;
            let min_ramp = (MIN_RAMP_SECS / (self.rate * spb)).min(0.2) as f32;
            let shape = self.shape.tick();
            let g =
                gate((beat / self.rate).rem_euclid(1.0) as f32, min_ramp.max(shape * 0.25), min_ramp.max(shape * 0.45));
            let k = 1.0 - self.depth.tick() * (1.0 - g);
            *f = [f[0] * k, f[1] * k];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::ctx;

    /// With DC input, full depth and the hardest shape, the first open frame
    /// of every period is the first frame at or after a multiple of the rate.
    #[test]
    fn gate_opens_on_the_grid() {
        for (knob, rate) in [(0.0, 0.25), (0.3, 0.125), (0.6, 0.0625), (0.9, 0.03125)] {
            for (bpm, start) in [(120.0, 0.0), (127.3, 3.1234567), (174.0, -1.77)] {
                let mut g = Gater::new(48_000.0);
                g.set_knob(0, knob);
                g.set_knob(1, 0.0);
                g.set_knob(2, 1.0);
                g.reset();
                let c = ctx(bpm, start);
                let mut buf = vec![[1.0f32; 2]; 96_000];
                for (i, b) in buf.chunks_mut(100).enumerate() {
                    g.process(b, &c.advanced(i * 100));
                }
                let bpf = c.beats_per_frame();
                let mut checked = 0;
                for n in 1..buf.len() {
                    if buf[n - 1][0] == 0.0 && buf[n][0] > 0.0 {
                        // Frame of the grid line at or before n.
                        let line = (c.beat_at(n) / rate).floor() * rate;
                        let want = ((line - start) / bpf).ceil() as usize;
                        assert!(n.abs_diff(want) <= 1, "rate {rate} bpm {bpm}: opened at {n}, grid at {want}");
                        checked += 1;
                    }
                }
                let expected = (96_000.0 * bpf / rate) as usize;
                assert!(checked + 2 >= expected, "{checked} of {expected} periods seen");
            }
        }
    }

    #[test]
    fn rate_change_waits_for_quarter_beat() {
        let mut g = Gater::new(48_000.0);
        g.set_knob(0, 0.0);
        let mut buf = vec![[1.0f32; 2]; 10];
        g.process(&mut buf, &ctx(120.0, 0.1));
        g.set_knob(0, 1.0);
        g.process(&mut buf, &ctx(120.0, 0.2));
        assert_eq!(g.rate, 0.25);
        g.process(&mut buf, &ctx(120.0, 0.25));
        assert_eq!(g.rate, 0.03125);
    }
}
