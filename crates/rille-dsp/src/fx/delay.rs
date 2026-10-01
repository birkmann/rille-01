//! Tempo-synced echo with a filter in the feedback path.

use super::{Effect, FxCtx, step};
use crate::clamp01;
use crate::delay_line::DelayLine;
use crate::denormal::undenormal;
use crate::filter::DjFilter;
use crate::limiter::SoftClip;
use crate::smooth::OnePole;

/// Delay times in beats.
const TIMES: [f64; 9] = [0.125, 0.1875, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0];
/// Buffer length: 4 beats at 60 bpm plus headroom.
const MAX_SECONDS: f32 = 4.1;
const MAX_FEEDBACK: f32 = 0.92;
/// Relative time change above which the delay crossfades to the new time
/// instead of gliding (tempo drift glides, knob steps crossfade).
const GLIDE_LIMIT: f32 = 0.03;
const GLIDE_MS: f32 = 60.0;
const XFADE_MS: f32 = 30.0;
const PARAM_MS: f32 = 20.0;

/// Echo synced to the master tempo. Knobs: time (1/8 … 4 beats), feedback,
/// filter in the feedback path (centre = off, left low-pass, right
/// high-pass). Button: freeze (repeat the buffer forever, no new input).
pub struct Delay {
    line: DelayLine,
    filter: DjFilter,
    beats: f64,
    feedback_knob: f32,
    filter_knob: f32,
    feedback: OnePole,
    input: OnePole,
    freeze: bool,
    /// Current delay in frames; `next` is the crossfade target.
    cur: f32,
    next: f32,
    fade: f32,
    fade_step: f32,
    fading: bool,
    glide: f32,
    fresh: bool,
}

impl Delay {
    pub fn new(sample_rate: f32) -> Self {
        let mut d = Self {
            line: DelayLine::new((MAX_SECONDS * sample_rate) as usize),
            filter: DjFilter::new(sample_rate),
            beats: 0.5,
            feedback_knob: 0.5,
            filter_knob: 0.5,
            feedback: OnePole::new(sample_rate, PARAM_MS, 0.0),
            input: OnePole::new(sample_rate, PARAM_MS, 1.0),
            freeze: false,
            cur: 1.0,
            next: 1.0,
            fade: 0.0,
            fade_step: 1.0 / (XFADE_MS * 0.001 * sample_rate),
            fading: false,
            glide: 1.0 - (-1.0 / (GLIDE_MS * 0.001 * sample_rate)).exp(),
            fresh: true,
        };
        d.apply_params();
        d.reset();
        d
    }

    fn apply_params(&mut self) {
        if self.freeze {
            self.feedback.set_target(1.0);
            self.input.set_target(0.0);
            self.filter.set_knob(0.5);
        } else {
            self.feedback.set_target(self.feedback_knob * MAX_FEEDBACK);
            self.input.set_target(1.0);
            self.filter.set_knob(self.filter_knob);
        }
    }
}

impl Effect for Delay {
    fn name(&self) -> &'static str {
        "Delay"
    }

    fn knob_names(&self) -> [&'static str; 3] {
        ["time", "feedback", "filter"]
    }

    fn button_names(&self) -> [&'static str; 3] {
        ["freeze", "", ""]
    }

    fn set_knob(&mut self, idx: usize, v: f32) {
        match idx {
            0 => self.beats = step(&TIMES, v),
            1 => self.feedback_knob = clamp01(v),
            2 => self.filter_knob = clamp01(v),
            _ => return,
        }
        self.apply_params();
    }

    fn set_button(&mut self, idx: usize, on: bool) {
        if idx == 0 {
            self.freeze = on;
            self.apply_params();
        }
    }

    fn reset(&mut self) {
        self.line.clear();
        self.filter.reset();
        self.feedback.set_immediate(self.feedback.target());
        self.input.set_immediate(self.input.target());
        self.fading = false;
        self.fresh = true;
    }

    fn process(&mut self, buf: &mut [[f32; 2]], ctx: &FxCtx) {
        // Whole frames: echoes stay sharp and freeze repeats without loss.
        let target = ((self.beats * ctx.frames_per_beat()).round() as f32).clamp(1.0, self.line.max_delay() as f32);
        if self.fresh {
            self.cur = target;
            self.fresh = false;
        }
        for f in buf.iter_mut() {
            if !self.fading {
                let diff = target - self.cur;
                if diff.abs() > self.cur * GLIDE_LIMIT {
                    self.next = target;
                    self.fade = 0.0;
                    self.fading = true;
                } else {
                    self.cur = if diff.abs() < 1e-3 { target } else { self.cur + diff * self.glide };
                }
            }
            let mut y = self.line.read(self.cur);
            if self.fading {
                let b = self.line.read(self.next);
                y = [y[0] + self.fade * (b[0] - y[0]), y[1] + self.fade * (b[1] - y[1])];
                self.fade += self.fade_step;
                if self.fade >= 1.0 {
                    self.cur = self.next;
                    self.fading = false;
                }
            }
            let fb = self.feedback.tick();
            let inp = self.input.tick();
            let fy = self.filter.tick(y);
            let w = |ch: usize| undenormal(SoftClip::shape(f[ch] * inp + fy[ch] * fb, 1.0, 2.0));
            self.line.write([w(0), w(1)]);
            *f = y;
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
    use crate::test_util::ctx;

    fn impulse_response(beats_knob: f32, bpm: f64, frames: usize) -> Vec<f32> {
        let mut d = Delay::new(48_000.0);
        d.set_knob(0, beats_knob);
        d.set_knob(1, 0.0);
        d.reset();
        let mut buf = vec![[0.0f32; 2]; frames];
        buf[0] = [1.0, 1.0];
        for (i, b) in buf.chunks_mut(128).enumerate() {
            d.process(b, &ctx(bpm, i as f64 * 128.0 * bpm / 60.0 / 48_000.0));
        }
        buf.iter().map(|f| f[0]).collect()
    }

    #[test]
    fn impulse_returns_after_synced_time() {
        for (knob, beats) in [(0.0, 0.125), (0.4, 0.375), (0.7, 1.0), (1.0, 4.0)] {
            for bpm in [60.0, 123.0, 174.5] {
                let want = (beats * 60.0 / bpm * 48_000.0f64).round() as usize;
                if want > (MAX_SECONDS * 48_000.0) as usize {
                    continue;
                }
                let out = impulse_response(knob, bpm, want + 1000);
                let peak = out.iter().enumerate().fold((0, 0f32), |m, (i, &v)| if v > m.1 { (i, v) } else { m });
                assert!(peak.0.abs_diff(want) <= 1, "{beats} beats @ {bpm}: {} vs {want}", peak.0);
                assert!(peak.1 > 0.99);
                assert!(out[..want - 1].iter().all(|v| v.abs() < 1e-6));
            }
        }
    }

    #[test]
    fn feedback_repeats_and_freeze_holds() {
        let mut d = Delay::new(48_000.0);
        d.set_knob(0, 0.7); // 1 beat = 24000 frames at 120 bpm
        d.set_knob(1, 0.5);
        d.reset();
        let mut buf = vec![[0.0f32; 2]; 24_000 * 3 + 10];
        buf[0] = [1.0, 1.0];
        d.process(&mut buf, &ctx(120.0, 0.0));
        let fb = 0.5 * MAX_FEEDBACK;
        assert!((buf[24_000][0] - 1.0).abs() < 1e-4);
        assert!((buf[48_000][0] - fb).abs() < 1e-3, "{}", buf[48_000][0]);
        assert!((buf[72_000][0] - fb * fb).abs() < 1e-3);
        // Freeze: the loop keeps its level and ignores new input.
        d.set_button(0, true);
        let mut buf = vec![[0.3f32; 2]; 24_000 * 4];
        d.process(&mut buf, &ctx(120.0, 3.0));
        let last = &buf[72_000..];
        let peak = last.iter().fold(0f32, |m, f| m.max(f[0].abs()));
        assert!(peak > 0.01 && peak < 0.5, "{peak}");
        let level: f32 = last.iter().map(|f| f[0].abs()).sum();
        let prev: f32 = buf[48_000..72_000].iter().map(|f| f[0].abs()).sum();
        assert!((level - prev).abs() < 1e-3 * prev.max(1e-3), "{level} vs {prev}");
    }

    #[test]
    fn time_change_does_not_click() {
        let mut d = Delay::new(48_000.0);
        d.set_knob(1, 0.3);
        let mut phase = 0.0f32;
        let mut prev = 0.0f32;
        let mut buf = vec![[0.0f32; 2]; 128];
        for block in 0..1500 {
            if block % 100 == 50 {
                d.set_knob(0, (block % 700) as f32 / 700.0);
            }
            for f in buf.iter_mut() {
                phase += 2.0 * std::f32::consts::PI * 100.0 / 48_000.0;
                *f = [phase.sin() * 0.5; 2];
            }
            d.process(&mut buf, &ctx(128.0, block as f64 * 128.0 / 22_500.0));
            for f in &buf {
                assert!((f[0] - prev).abs() < 0.05, "block {block}: {prev} -> {}", f[0]);
                prev = f[0];
            }
        }
    }
}
