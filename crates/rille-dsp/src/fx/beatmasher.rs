//! Beat-locked stutter loop.

use super::{Effect, FxCtx, step};
use crate::clamp01;
use crate::smooth::OnePole;

/// Loop lengths in beats.
const LENGTHS: [f64; 8] = [1.0 / 32.0, 1.0 / 16.0, 1.0 / 8.0, 0.25, 0.5, 1.0, 2.0, 4.0];
/// Longest loop; the capture window is aligned to multiples of it.
const WINDOW_BEATS: f64 = 4.0;
/// Buffer: 4 beats at 60 bpm plus headroom.
const BUFFER_SECS: f32 = 4.5;
const XFADE_MS: f32 = 2.0;
/// Beat discontinuity (seek, resync) that makes an active loop re-arm.
const JUMP_BEATS: f64 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Pass-through.
    Off,
    /// Pass-through until the next beat boundary, then loop.
    Armed,
    Active,
}

/// Captures the input from the next beat boundary and repeats a segment of
/// the chosen length, locked to the master clock: segments restart exactly at
/// multiples of their length in `beat_pos`.
///
/// Input is recorded continuously while not looping, so loops of 2 and 4
/// beats can start at the enclosing multiple of their length even when they
/// engage mid-bar; the audio before the engage point comes from that
/// recording. Discontinuities (restarts, length or direction changes,
/// engaging and releasing) are crossfaded over 2 ms.
///
/// Knobs: length (1/32 … 4 beats), pitch (±1 octave by read speed, centre =
/// off). Buttons: on (arm when pressed, pass-through when released; `reset`
/// also arms, so selecting the effect or switching the unit on engages it),
/// reverse.
pub struct Beatmasher {
    buf: Box<[[f32; 2]]>,
    /// Absolute index of the next frame.
    frame: u64,
    /// Absolute frames `rec_start..rec_end` are in the buffer.
    rec_start: u64,
    rec_end: u64,
    state: State,
    /// Beat at which the loop engages (NaN until the first armed frame).
    engage_beat: f64,
    /// Recording window `[win_start, win_start + win_len)` in beats.
    win_start: f64,
    win_len: f64,
    /// Beat-to-frame mapping taken when the loop engaged.
    anchor_frame: f64,
    anchor_beat: f64,
    fpb: f64,
    length: f64,
    reverse: bool,
    pitch: OnePole,
    /// Segment index of the previous frame and the read offset in it.
    seg: i64,
    loop_pos: f64,
    last_beat: f64,
    last_src: f64,
    last_rate: f64,
    fade_len: u32,
    fade_left: u32,
    fade_pos: f64,
    fade_rate: f64,
}

impl Beatmasher {
    pub fn new(sample_rate: f32) -> Self {
        let mut b = Self {
            buf: vec![[0.0; 2]; (BUFFER_SECS * sample_rate) as usize].into_boxed_slice(),
            frame: 0,
            rec_start: 0,
            rec_end: 0,
            state: State::Armed,
            engage_beat: f64::NAN,
            win_start: 0.0,
            win_len: WINDOW_BEATS,
            anchor_frame: 0.0,
            anchor_beat: 0.0,
            fpb: 1.0,
            length: 0.25,
            reverse: false,
            pitch: OnePole::new(sample_rate, 20.0, 1.0),
            seg: i64::MIN,
            loop_pos: 0.0,
            last_beat: f64::NAN,
            last_src: f64::NAN,
            last_rate: 1.0,
            fade_len: ((XFADE_MS * 0.001 * sample_rate) as u32).max(1),
            fade_left: 0,
            fade_pos: 0.0,
            fade_rate: 1.0,
        };
        b.set_knob(0, 0.4);
        b.reset();
        b
    }

    fn arm(&mut self) {
        self.state = State::Armed;
        self.engage_beat = f64::NAN;
    }

    fn engage(&mut self, beat: f64, ctx: &FxCtx) {
        self.fpb = ctx.frames_per_beat();
        // Largest power-of-two window (≤ 4 beats) that fits the buffer at this tempo.
        let room = (self.buf.len() - 2) as f64;
        self.win_len = WINDOW_BEATS;
        while self.win_len * self.fpb > room && self.win_len > LENGTHS[0] {
            self.win_len /= 2.0;
        }
        self.win_start = (self.engage_beat / self.win_len).floor() * self.win_len;
        self.anchor_frame = self.frame as f64;
        self.anchor_beat = beat;
        self.seg = i64::MIN;
        self.state = State::Active;
    }

    fn frame_of(&self, beat: f64) -> f64 {
        self.anchor_frame + (beat - self.anchor_beat) * self.fpb
    }

    /// Recorded frame at fractional absolute position `pos` (linear
    /// interpolation); `live` for positions not recorded yet, silence for
    /// positions already overwritten.
    fn read(&self, pos: f64, live: [f32; 2]) -> [f32; 2] {
        let hi = self.rec_end as f64 - 1.0;
        if self.rec_end == self.rec_start || pos > hi + 0.5 {
            return live;
        }
        if pos < self.rec_start as f64 {
            return [0.0; 2];
        }
        let p = pos.min(hi);
        let i = p as u64;
        let t = (p - i as f64) as f32;
        let n = self.buf.len() as u64;
        let a = self.buf[(i % n) as usize];
        let b = if i + 1 < self.rec_end { self.buf[((i + 1) % n) as usize] } else { a };
        [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
    }

    /// Source position (absolute frame) and read direction/speed for the
    /// current frame; `None` means live input.
    fn source(&mut self, beat: f64) -> Option<(f64, f64)> {
        if self.state != State::Active {
            return None;
        }
        let len = self.length.min(self.win_len);
        let start = (self.engage_beat / len).floor() * len;
        if beat < start + len {
            // First pass: the segment is still being recorded.
            self.seg = i64::MIN;
            return None;
        }
        let seg = (beat / len).floor() as i64;
        if seg != self.seg {
            self.seg = seg;
            self.loop_pos = (beat - seg as f64 * len) * self.fpb;
        }
        let seg_frames = len * self.fpb;
        let p = self.loop_pos.rem_euclid(seg_frames);
        let rate = self.pitch.tick() as f64;
        self.loop_pos += rate;
        let base = self.frame_of(start);
        Some(if self.reverse { (base + seg_frames - 1.0 - p, -rate) } else { (base + p, rate) })
    }
}

impl Effect for Beatmasher {
    fn name(&self) -> &'static str {
        "Beatmasher"
    }

    fn knob_names(&self) -> [&'static str; 3] {
        ["length", "pitch", ""]
    }

    fn button_names(&self) -> [&'static str; 3] {
        ["on", "reverse", ""]
    }

    fn set_knob(&mut self, idx: usize, v: f32) {
        match idx {
            0 => self.length = step(&LENGTHS, v),
            1 => {
                let d = clamp01(v) - 0.5;
                self.pitch.set_target(if d.abs() < 0.02 { 1.0 } else { (d * 2.0).exp2() });
            }
            _ => {}
        }
    }

    fn set_button(&mut self, idx: usize, on: bool) {
        match (idx, on) {
            (0, true) => self.arm(),
            (0, false) => self.state = State::Off,
            (1, _) => self.reverse = on,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.rec_start = self.frame;
        self.rec_end = self.frame;
        self.pitch.set_immediate(self.pitch.target());
        self.last_beat = f64::NAN;
        self.last_src = f64::NAN;
        self.fade_left = 0;
        self.arm();
    }

    fn process(&mut self, buf: &mut [[f32; 2]], ctx: &FxCtx) {
        let bpf = ctx.beats_per_frame();
        let cap = self.buf.len() as u64;
        for (n, f) in buf.iter_mut().enumerate() {
            let beat = ctx.beat_at(n);
            let x = *f;
            if self.state == State::Active && (beat - self.last_beat - bpf).abs() > JUMP_BEATS {
                self.arm();
            }
            self.last_beat = beat;
            if self.state != State::Active || beat < self.win_start + self.win_len {
                self.buf[(self.frame % cap) as usize] = x;
                self.rec_end = self.frame + 1;
                self.rec_start = self.rec_start.max(self.rec_end.saturating_sub(cap));
            }
            if self.state == State::Armed {
                if self.engage_beat.is_nan() {
                    self.engage_beat = beat.ceil();
                }
                if beat >= self.engage_beat {
                    self.engage(beat, ctx);
                }
            }
            let (src, rate, mut y) = match self.source(beat) {
                Some((src, rate)) => (src, rate, self.read(src, x)),
                None => (self.frame as f64, 1.0, x),
            };
            let expected = self.last_src + self.last_rate;
            if (src - expected).abs() > 1.0 {
                self.fade_left = self.fade_len;
                self.fade_pos = expected;
                self.fade_rate = self.last_rate;
            }
            if self.fade_left > 0 {
                let c = self.read(self.fade_pos, x);
                let w = self.fade_left as f32 / self.fade_len as f32;
                y = [y[0] + w * (c[0] - y[0]), y[1] + w * (c[1] - y[1])];
                self.fade_pos += self.fade_rate;
                self.fade_left -= 1;
            }
            self.last_src = src;
            self.last_rate = rate;
            self.frame += 1;
            *f = y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::ctx;

    const SCALE: f32 = 1e-6;

    /// Runs a ramp (value = frame index · SCALE) through the effect, so the
    /// output tells which source frame was played.
    fn run(length_knob: f32, reverse: bool, bpm: f64, beat0: f64, frames: usize) -> (Vec<f64>, FxCtx) {
        let mut b = Beatmasher::new(48_000.0);
        b.set_knob(0, length_knob);
        b.set_button(1, reverse);
        b.reset();
        let mut buf: Vec<[f32; 2]> = (0..frames).map(|i| [i as f32 * SCALE; 2]).collect();
        let c = ctx(bpm, beat0);
        for (i, blk) in buf.chunks_mut(97).enumerate() {
            b.process(blk, &c.advanced(i * 97));
        }
        (buf.iter().map(|f| (f[0] / SCALE) as f64).collect(), c)
    }

    #[test]
    fn segments_restart_on_multiples_of_length() {
        for (knob, len) in [(0.0, 1.0 / 32.0), (0.4, 0.25), (0.7, 1.0), (0.9, 4.0)] {
            for (bpm, beat0) in [(120.0, 0.3), (128.7, 5.61)] {
                let (src, c) = run(knob, false, bpm, beat0, 48_000 * 6);
                let fpb = c.frames_per_beat();
                let engage = beat0.ceil();
                let start = (engage / len).floor() * len;
                let fade = 96;
                let mut checked = 0;
                for (n, &s) in src.iter().enumerate() {
                    let beat = c.beat_at(n);
                    let seg = (beat / len).floor() * len;
                    let since = ((beat - seg) * fpb) as usize;
                    // Live (pass-through) until the first segment has been played once.
                    let want = if beat < start + len {
                        n as f64
                    } else if since <= fade || since + 2 > (len * fpb) as usize {
                        continue; // crossfade zone around the restart
                    } else {
                        (start - beat0) * fpb + (beat - seg) * fpb
                    };
                    if want < 0.0 {
                        assert_eq!(s, 0.0, "audio from before the recording started must be silent");
                        continue;
                    }
                    assert!((s - want).abs() <= 1.0, "len {len} bpm {bpm}: frame {n} played {s}, want {want}");
                    checked += 1;
                }
                assert!(checked > 100_000);
            }
        }
    }

    #[test]
    fn reverse_plays_segment_backwards() {
        let (src, c) = run(0.5, true, 120.0, 0.0, 48_000 * 2); // 1/2 beat = 12000 frames
        let fpb = c.frames_per_beat();
        // Engages at beat 0: first pass live, then each half beat plays frames 11999..0.
        assert!((src[6000] - 6000.0).abs() < 1.0);
        for k in 1..6 {
            let n = (k as f64 * 0.5 * fpb) as usize;
            assert!((src[n + 1000] - (12_000.0 - 1.0 - 1000.0)).abs() <= 1.5, "{}", src[n + 1000]);
            assert!((src[n + 11_000] - (12_000.0 - 1.0 - 11_000.0)).abs() <= 1.5);
        }
    }

    #[test]
    fn off_is_pass_through_and_on_rearms() {
        let mut b = Beatmasher::new(48_000.0);
        b.set_button(0, false);
        let input: Vec<[f32; 2]> = (0..48_000).map(|i| [(i as f32 * 0.01).sin(); 2]).collect();
        let mut buf = input.clone();
        b.process(&mut buf, &ctx(120.0, 0.5));
        assert_eq!(buf, input);
        b.set_button(0, true);
        assert_eq!(b.state, State::Armed);
    }
}
