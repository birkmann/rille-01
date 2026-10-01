//! Feedback delay network reverb.

use super::{Effect, FxCtx};
use crate::clamp01;
use crate::delay_line::DelayLine;
use crate::denormal::undenormal;
use crate::smooth::OnePole;

/// FDN line lengths in ms (mutually prime-ish at common sample rates).
const LINE_MS: [f32; 8] = [31.7, 37.3, 41.9, 47.1, 53.3, 59.9, 67.3, 73.7];
/// Input diffusers per channel, ms.
const DIFFUSER_MS: [[f32; 4]; 2] = [[4.7, 3.6, 12.7, 9.3], [5.3, 3.3, 11.9, 8.7]];
const DIFFUSION: f32 = 0.6;
const MAX_PREDELAY_MS: f32 = 200.0;
const MIN_RT60: f32 = 0.3;
const MAX_RT60: f32 = 12.0;
const DAMP_MAX_HZ: f32 = 16_000.0;
const DAMP_MIN_HZ: f32 = 1000.0;
/// Loop gain while frozen: practically infinite, but float drift cannot grow.
const FREEZE_GAIN: f32 = 1.0 - 1e-5;
const IN_GAIN: f32 = 0.35;
const OUT_GAIN: f32 = 0.6;
const SUB_BLOCK: u32 = 16;
const PARAM_MS: f32 = 50.0;

/// Mono ring buffer of a fixed delay.
#[derive(Clone, Debug)]
struct Line {
    buf: Box<[f32]>,
    pos: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)].into_boxed_slice(), pos: 0 }
    }

    /// The sample written `len` frames ago; the next `write` replaces it.
    #[inline]
    fn read(&self) -> f32 {
        self.buf[self.pos]
    }

    #[inline]
    fn write(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = if self.pos + 1 == self.buf.len() { 0 } else { self.pos + 1 };
    }

    /// Schroeder all-pass step.
    #[inline]
    fn allpass(&mut self, x: f32, g: f32) -> f32 {
        let v = self.read();
        let w = undenormal(x + g * v);
        self.write(w);
        v - g * w
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// 8-line FDN reverb with Householder mixing and input diffusion. Knobs:
/// size (decay 0.3–12 s), damping (high cut in the loop), pre-delay
/// (0–200 ms). Button: freeze (infinite sustain, no new input).
pub struct Reverb {
    sample_rate: f32,
    lines: [Line; 8],
    damp: [f32; 8],
    diffusers: [[Line; 4]; 2],
    predelay: DelayLine,
    rt60: OnePole,
    damp_hz: OnePole,
    predelay_frames: OnePole,
    freeze: OnePole,
    freeze_on: bool,
    gains: [f32; 8],
    damp_coef: f32,
    left: u32,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let frames = |ms: f32| (ms * 0.001 * sample_rate).round() as usize;
        let block_rate = sample_rate / SUB_BLOCK as f32;
        let mut r = Self {
            sample_rate,
            lines: LINE_MS.map(|ms| Line::new(frames(ms))),
            damp: [0.0; 8],
            diffusers: DIFFUSER_MS.map(|ch| ch.map(|ms| Line::new(frames(ms)))),
            predelay: DelayLine::new(frames(MAX_PREDELAY_MS) + 1),
            rt60: OnePole::new(block_rate, PARAM_MS, 0.0),
            damp_hz: OnePole::new(block_rate, PARAM_MS, 0.0),
            predelay_frames: OnePole::new(sample_rate, PARAM_MS, 0.0),
            freeze: OnePole::new(sample_rate, 20.0, 0.0),
            freeze_on: false,
            gains: [0.0; 8],
            damp_coef: 1.0,
            left: 0,
        };
        r.set_knob(0, 0.5);
        r.set_knob(1, 0.3);
        r.set_knob(2, 0.1);
        r.reset();
        r
    }

    fn update(&mut self) {
        self.left = SUB_BLOCK;
        let rt = self.rt60.tick();
        for (g, l) in self.gains.iter_mut().zip(&self.lines) {
            *g = 10f32.powf(-3.0 * l.buf.len() as f32 / (self.sample_rate * rt));
        }
        let hz = self.damp_hz.tick();
        self.damp_coef = 1.0 - (-2.0 * std::f32::consts::PI * hz / self.sample_rate).exp();
    }
}

impl Effect for Reverb {
    fn name(&self) -> &'static str {
        "Reverb"
    }

    fn knob_names(&self) -> [&'static str; 3] {
        ["size", "damping", "predelay"]
    }

    fn button_names(&self) -> [&'static str; 3] {
        ["freeze", "", ""]
    }

    fn set_knob(&mut self, idx: usize, v: f32) {
        let v = clamp01(v);
        match idx {
            0 => self.rt60.set_target(MIN_RT60 * (MAX_RT60 / MIN_RT60).powf(v)),
            1 => self.damp_hz.set_target(DAMP_MAX_HZ * (DAMP_MIN_HZ / DAMP_MAX_HZ).powf(v)),
            2 => self.predelay_frames.set_target(1.0 + v * MAX_PREDELAY_MS * 0.001 * self.sample_rate),
            _ => {}
        }
    }

    fn set_button(&mut self, idx: usize, on: bool) {
        if idx == 0 {
            self.freeze_on = on;
            self.freeze.set_target(if on { 1.0 } else { 0.0 });
        }
    }

    fn reset(&mut self) {
        self.lines.iter_mut().chain(self.diffusers.iter_mut().flatten()).for_each(Line::clear);
        self.damp = [0.0; 8];
        self.predelay.clear();
        for p in [&mut self.rt60, &mut self.damp_hz, &mut self.predelay_frames, &mut self.freeze] {
            p.set_immediate(p.target());
        }
        self.left = 0;
    }

    fn process(&mut self, buf: &mut [[f32; 2]], _ctx: &FxCtx) {
        for f in buf.iter_mut() {
            if self.left == 0 {
                self.update();
            }
            self.left -= 1;
            let pd = self.predelay.read(self.predelay_frames.tick());
            self.predelay.write(*f);
            let fz = self.freeze.tick();
            let mut inp = [pd[0] * (1.0 - fz), pd[1] * (1.0 - fz)];
            for (x, chain) in inp.iter_mut().zip(&mut self.diffusers) {
                for ap in chain.iter_mut() {
                    *x = ap.allpass(*x, DIFFUSION);
                }
            }
            let v: [f32; 8] = std::array::from_fn(|i| self.lines[i].read());
            let dc = self.damp_coef + (1.0 - self.damp_coef) * fz;
            let mut d = [0.0f32; 8];
            for i in 0..8 {
                self.damp[i] = undenormal(self.damp[i] + (v[i] - self.damp[i]) * dc);
                d[i] = self.damp[i] * (self.gains[i] + (FREEZE_GAIN - self.gains[i]) * fz);
            }
            // Householder reflection: orthogonal, so the loop is lossless before the gains.
            let s = d.iter().sum::<f32>() * (2.0 / 8.0);
            for i in 0..8 {
                let u = d[i] - s + inp[i & 1] * IN_GAIN;
                self.lines[i].write(undenormal(u));
            }
            *f = [(v[0] - v[2] + v[4] - v[6]) * OUT_GAIN, (v[1] - v[3] + v[5] - v[7]) * OUT_GAIN];
        }
    }

    fn has_tail(&self) -> bool {
        true
    }

    fn release(&mut self) {
        self.set_button(0, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Rng, ctx};

    fn energy(buf: &[[f32; 2]]) -> f32 {
        buf.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum::<f32>() / buf.len() as f32
    }

    #[test]
    fn decays_by_rt60_and_level_is_sane() {
        let mut r = Reverb::new(48_000.0);
        r.set_knob(0, 0.5); // ~1.9 s
        r.set_knob(1, 0.0);
        r.reset();
        let rt = r.rt60.value();
        let mut rng = Rng::new(2);
        // Low-passed noise, so the (frequency-dependent) damping barely matters.
        let mut lp = [OnePole::new(48_000.0, 0.2, 0.0), OnePole::new(48_000.0, 0.2, 0.0)];
        let mut noise: Vec<[f32; 2]> =
            (0..48_000).map(|_| [lp[0].filter(rng.bipolar()), lp[1].filter(rng.bipolar())]).collect();
        let in_energy = energy(&noise);
        r.process(&mut noise, &ctx(120.0, 0.0));
        let wet = energy(&noise[24_000..]);
        assert!(wet > in_energy * 0.1 && wet < in_energy * 10.0, "{wet} vs {in_energy}");
        // Tail: energy drop over `rt/2` seconds should be ~30 dB.
        let n = (rt * 0.5 * 48_000.0) as usize;
        let mut tail = vec![[0.0f32; 2]; 12_000 + n];
        r.process(&mut tail, &ctx(120.0, 0.0));
        let drop = 10.0 * (energy(&tail[..4800]) / energy(&tail[n..n + 4800])).log10();
        assert!((drop - 30.0).abs() < 5.0, "{drop} dB over rt/2");
    }

    #[test]
    fn freeze_sustains() {
        let mut r = Reverb::new(48_000.0);
        r.set_knob(0, 0.0); // shortest decay
        let mut rng = Rng::new(4);
        let mut buf: Vec<[f32; 2]> = (0..24_000).map(|_| [rng.bipolar(), rng.bipolar()]).collect();
        r.process(&mut buf, &ctx(120.0, 0.0));
        r.set_button(0, true);
        let mut buf = vec![[0.5f32; 2]; 48_000 * 3];
        r.process(&mut buf, &ctx(120.0, 0.0));
        let a = energy(&buf[24_000..48_000]);
        let b = energy(&buf[120_000..]);
        assert!(a > 1e-3 && (b / a) > 0.9, "{a} {b}");
        r.release();
        let mut buf = vec![[0.0f32; 2]; 48_000 * 3];
        r.process(&mut buf, &ctx(120.0, 0.0));
        assert!(energy(&buf[120_000..]) < 1e-9);
    }
}
