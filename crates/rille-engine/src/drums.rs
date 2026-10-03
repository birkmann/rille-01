//! Drum machine: a 16-step sequencer over eight one-shot sample voices,
//! always in time with the master clock.
//!
//! Steps sit on a lattice of sixteenths counted from the clock's bar origin
//! (the leading track's downbeat). Each block the machine fires the steps
//! whose time falls between the end of the last block and the end of this
//! one, on the exact frame. It counts steps with an absolute index that only
//! moves forward, so small corrections of the clock never fire a step twice
//! or skip one, and a jump of the clock (a new leader, a seek) re-syncs
//! without a burst of hits.
//!
//! The clock beat at the end of a block is the beat heard from the leader at
//! that point (its keylock latency is already taken out), so hits need no
//! latency offset to line up with it.

use std::sync::Arc;

use rille_core::drums::{CELLS, CHOKE, INSTRUMENTS, NORMAL_GAIN, PATTERNS, Pattern, STEPS, pattern_step, step_time};
use rille_core::{Control, ControlValue};

use crate::mixer::{Strip, fader_gain};
use crate::types::TrackAudio;

/// Release time of a voice that is cut (choke, retrigger, kit change).
const RELEASE_SECS: f32 = 0.005;
/// Steps up to this many beats before the block are still played (late
/// rather than lost after a small backward correction of the clock).
const CATCH_UP_BEATS: f64 = 1.0 / 32.0;
/// Tune knob range, semitones either way.
const TUNE_SEMITONES: f64 = 12.0;

/// A set of eight samples, one per instrument. Built on the app side and
/// sent with `Command::SetDrumKit`.
#[derive(Default)]
pub struct DrumKit {
    pub samples: [Option<Arc<TrackAudio>>; INSTRUMENTS],
}

impl std::fmt::Debug for DrumKit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let loaded = self.samples.iter().filter(|s| s.is_some()).count();
        write!(f, "DrumKit({loaded} samples)")
    }
}

/// Settings restored by the app (`Command::SetDrumParams`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrumParams {
    pub level: [f32; INSTRUMENTS],
    pub tune: [f32; INSTRUMENTS],
    pub decay: [f32; INSTRUMENTS],
    pub muted: [bool; INSTRUMENTS],
    pub current: u8,
    pub selected: u8,
    pub channel_level: f32,
    pub filter: f32,
    pub fx_assign: [bool; 2],
}

impl Default for DrumParams {
    fn default() -> Self {
        Self {
            level: [1.0; INSTRUMENTS],
            tune: [0.5; INSTRUMENTS],
            decay: [1.0; INSTRUMENTS],
            muted: [false; INSTRUMENTS],
            current: 0,
            selected: 0,
            channel_level: 0.8,
            filter: 0.5,
            fx_assign: [false; 2],
        }
    }
}

/// One sounding hit.
#[derive(Clone, Copy, Default)]
struct DrumVoice {
    active: bool,
    /// Reads the previous kit (it is fading out after a kit change).
    old_kit: bool,
    pos: f64,
    rate: f64,
    gain: f32,
    /// Decay envelope: level now and factor per frame.
    env: f32,
    env_mul: f32,
    /// Release: level now and step per frame (0 = not releasing).
    release: f32,
    release_step: f32,
}

impl DrumVoice {
    fn cut(&mut self, frames: usize) {
        if self.active && self.release_step == 0.0 {
            self.release = 1.0;
            self.release_step = 1.0 / frames.max(1) as f32;
        }
    }

    /// Adds `out.len()` frames of `audio` into `out`.
    fn render(&mut self, audio: &TrackAudio, level: f32, out: &mut [[f32; 2]]) {
        let frames = &audio.frames;
        let last = frames.len().saturating_sub(1);
        for o in out.iter_mut() {
            if self.pos >= last as f64 || (self.release_step > 0.0 && self.release <= 0.0) {
                self.active = false;
                return;
            }
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            let at = |k: isize| frames[(i as isize + k).clamp(0, last as isize) as usize];
            let s = hermite(at(-1), at(0), at(1), at(2), t);
            let mut g = self.gain * level * self.env;
            if self.release_step > 0.0 {
                g *= self.release;
                self.release -= self.release_step;
            }
            o[0] += s[0] * g;
            o[1] += s[1] * g;
            self.env *= self.env_mul;
            self.pos += self.rate;
        }
        if self.env < 1e-4 {
            self.active = false;
        }
    }
}

/// Cubic Hermite interpolation between `b` and `c`.
fn hermite(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2], t: f32) -> [f32; 2] {
    std::array::from_fn(|ch| {
        let (a, b, c, d) = (a[ch], b[ch], c[ch], d[ch]);
        let c1 = 0.5 * (c - a);
        let c2 = a - 2.5 * b + 2.0 * c - 0.5 * d;
        let c3 = 0.5 * (d - a) + 1.5 * (b - c);
        ((c3 * t + c2) * t + c1) * t + b
    })
}

/// Per-frame factor of the decay envelope for knob `decay`: 1.0 keeps the
/// whole sample, lower values fade it out over 10 ms … 3 s.
fn decay_mul(decay: f32, sr: f64) -> f32 {
    if decay >= 0.999 {
        return 1.0;
    }
    let tau = 0.01 * 300f64.powf(f64::from(decay.clamp(0.0, 1.0)));
    (-1.0 / (tau * sr)).exp() as f32
}

pub struct DrumMachine {
    sr: f64,
    kit: Option<Arc<DrumKit>>,
    /// The kit before the last change, while its hits fade out.
    old_kit: Option<Arc<DrumKit>>,
    /// Two voices per instrument (a retrigger lets the last hit fade).
    voices: [[DrumVoice; 2]; INSTRUMENTS],
    pub(crate) patterns: [Pattern; PATTERNS],
    current: usize,
    queued: Option<usize>,
    selected: usize,
    playing: bool,
    record: bool,
    level: [f32; INSTRUMENTS],
    tune: [f32; INSTRUMENTS],
    decay: [f32; INSTRUMENTS],
    muted: [bool; INSTRUMENTS],
    hits: [u32; INSTRUMENTS],
    /// Instruments to play at the start of the next block (triggers).
    pending: u8,
    /// Step each instrument must not play again (it was just recorded and
    /// played live).
    skip: [i64; INSTRUMENTS],
    /// Clock beat at the end of the last block.
    last_end: f64,
    /// Bar origin the step lattice was counted from.
    origin: f64,
    /// Next absolute step to fire.
    next_step: i64,
    /// Find `next_step` afresh: at the block start (just started) or end.
    resync: Option<Resync>,
    /// The machine just started: the engine should take the leader's
    /// downbeat at once.
    pub(crate) origin_request: bool,
    /// Bumped by every change worth saving.
    edit_rev: u32,
    pub(crate) strip: Strip,
    pub(crate) buf: Vec<[f32; 2]>,
}

#[derive(Clone, Copy, PartialEq)]
enum Resync {
    FromStart,
    FromEnd,
}

impl DrumMachine {
    pub fn new(sample_rate: u32, max_block: usize) -> Self {
        let mut strip = Strip::new(sample_rate as f32);
        strip.volume = 0.8;
        Self {
            sr: f64::from(sample_rate),
            kit: None,
            old_kit: None,
            voices: [[DrumVoice::default(); 2]; INSTRUMENTS],
            patterns: rille_core::drums::factory_patterns(),
            current: 0,
            queued: None,
            selected: 0,
            playing: false,
            record: false,
            level: [1.0; INSTRUMENTS],
            tune: [0.5; INSTRUMENTS],
            decay: [1.0; INSTRUMENTS],
            muted: [false; INSTRUMENTS],
            hits: [0; INSTRUMENTS],
            pending: 0,
            skip: [i64::MIN; INSTRUMENTS],
            last_end: 0.0,
            origin: 0.0,
            next_step: 0,
            resync: Some(Resync::FromEnd),
            origin_request: false,
            edit_rev: 0,
            strip,
            buf: vec![[0.0; 2]; max_block],
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    fn release_frames(&self) -> usize {
        (f64::from(RELEASE_SECS) * self.sr) as usize
    }

    fn edited(&mut self) {
        self.edit_rev = self.edit_rev.wrapping_add(1);
    }

    /// Installs `kit`; returns the kit it replaces for the garbage queue (and
    /// a kit that could not wait for its hits to fade out).
    pub(crate) fn set_kit(&mut self, kit: Option<Arc<DrumKit>>) -> [Option<Arc<DrumKit>>; 2] {
        let release = self.release_frames();
        let mut dropped = [None, None];
        // A kit still fading from the last change goes now, silently.
        if let Some(old) = self.old_kit.take() {
            for v in self.voices.iter_mut().flatten().filter(|v| v.old_kit) {
                v.active = false;
            }
            dropped[0] = Some(old);
        }
        for v in self.voices.iter_mut().flatten().filter(|v| v.active) {
            v.old_kit = true;
            v.cut(release);
        }
        self.old_kit = std::mem::replace(&mut self.kit, kit);
        if self.voices.iter().flatten().all(|v| !v.active) {
            dropped[1] = self.old_kit.take();
        }
        dropped
    }

    /// The old kit once its last hit has faded out.
    pub(crate) fn take_faded_kit(&mut self) -> Option<Arc<DrumKit>> {
        if self.old_kit.is_some() && self.voices.iter().flatten().all(|v| !(v.active && v.old_kit)) {
            self.old_kit.take()
        } else {
            None
        }
    }

    pub(crate) fn set_pattern(&mut self, index: usize, pattern: Pattern) {
        if let Some(p) = self.patterns.get_mut(index) {
            *p = pattern;
            self.edited();
        }
    }

    pub(crate) fn set_params(&mut self, p: DrumParams) {
        self.level = p.level;
        self.tune = p.tune;
        self.decay = p.decay;
        self.muted = p.muted;
        self.current = usize::from(p.current).min(PATTERNS - 1);
        self.queued = None;
        self.selected = usize::from(p.selected).min(INSTRUMENTS - 1);
        self.strip.volume = p.channel_level;
        self.strip.filter = p.filter;
        self.strip.fx_assign = p.fx_assign;
        self.edited();
    }

    /// Starts instrument `inst` now (at the start of the next block).
    fn trigger(&mut self, inst: usize) {
        self.pending |= 1 << inst;
        if self.playing && self.record {
            // Write the step nearest to what is heard now and keep the
            // sequencer from playing it again right after.
            let pos = self.last_end - self.origin;
            let swing = self.patterns[self.current].swing;
            let s0 = (pos * 4.0).floor() as i64;
            let s =
                if (step_time(s0 + 1, swing) - pos).abs() < (pos - step_time(s0, swing)).abs() { s0 + 1 } else { s0 };
            let pat = &mut self.patterns[self.current];
            pat.set(inst, pattern_step(s, pat.length()), true);
            if s >= self.next_step {
                self.skip[inst] = s;
            }
            self.edited();
        }
    }

    /// Starts a hit of `inst` at full level × `gain`.
    fn start_voice(&mut self, inst: usize, gain: f32) {
        self.hits[inst] = self.hits[inst].wrapping_add(1);
        if self.muted[inst] {
            return;
        }
        let Some(audio) = self.kit.as_ref().and_then(|k| k.samples[inst].as_ref()) else { return };
        let rate = 2f64.powf((f64::from(self.tune[inst]) - 0.5) * 2.0 * TUNE_SEMITONES / 12.0)
            * f64::from(audio.sample_rate)
            / self.sr;
        let release = self.release_frames();
        if inst == CHOKE.0 {
            for v in &mut self.voices[CHOKE.1] {
                v.cut(release);
            }
        }
        let pair = &mut self.voices[inst];
        // Take a free voice, or the one that is already releasing.
        let k = if !pair[0].active {
            0
        } else if !pair[1].active {
            1
        } else if pair[0].release_step > 0.0 {
            0
        } else {
            1
        };
        pair[1 - k].cut(release);
        pair[k] = DrumVoice {
            active: true,
            old_kit: false,
            pos: 0.0,
            rate,
            gain,
            env: 1.0,
            env_mul: decay_mul(self.decay[inst], self.sr),
            release: 1.0,
            release_step: 0.0,
        };
    }

    /// Fires absolute step `s` of the sequence.
    fn fire_step(&mut self, s: i64) {
        let len = self.patterns[self.current].length();
        let step = pattern_step(s, len);
        if step == 0
            && let Some(q) = self.queued.take()
        {
            self.current = q;
            self.edited();
        }
        let pat = self.patterns[self.current];
        let step = pattern_step(s, pat.length());
        for inst in 0..INSTRUMENTS {
            if pat.is_on(inst, step) && self.skip[inst] != s {
                let gain = if pat.is_accent(inst, step) { 1.0 } else { NORMAL_GAIN };
                self.start_voice(inst, gain);
            }
        }
    }

    /// Renders voices into `buf[from..to]`.
    fn render_voices(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        for inst in 0..INSTRUMENTS {
            let level = fader_gain(self.level[inst]);
            for v in &mut self.voices[inst] {
                if !v.active {
                    continue;
                }
                let kit = if v.old_kit { &self.old_kit } else { &self.kit };
                match kit.as_ref().and_then(|k| k.samples[inst].as_deref()) {
                    Some(audio) => v.render(audio, level, &mut self.buf[from..to]),
                    None => v.active = false,
                }
            }
        }
    }

    /// First absolute step at or after `pos` beats from the origin.
    fn first_step_from(pos: f64, swing: f32) -> i64 {
        let mut s = (pos * 4.0).floor() as i64 - 1;
        while step_time(s, swing) < pos - 1e-9 {
            s += 1;
        }
        s
    }

    /// Renders `n` frames into `self.buf`. `clock_end` is the master clock's
    /// beat at the end of the block, `origin` its bar origin, `bpm` its tempo.
    pub(crate) fn render(&mut self, n: usize, clock_end: f64, origin: f64, bpm: f64) {
        let n = n.min(self.buf.len());
        self.buf[..n].fill([0.0; 2]);
        let (b0, b1) = (self.last_end, clock_end);
        self.last_end = b1;
        self.origin_request = false;
        // The origin moved (a new downbeat): keep counting on the same lattice.
        if origin != self.origin {
            self.next_step += ((self.origin - origin) * 4.0).round() as i64;
            self.origin = origin;
        }
        let swing = self.patterns[self.current].swing;
        let mut done = 0;
        if self.pending != 0 {
            let pending = std::mem::take(&mut self.pending);
            for inst in (0..INSTRUMENTS).filter(|i| pending & (1 << i) != 0) {
                self.start_voice(inst, 1.0);
            }
        }
        if self.playing {
            let d = b1 - b0;
            let nominal = bpm.max(1.0) / 60.0 * n as f64 / self.sr;
            let resync = match self.resync.take() {
                Some(r) => Some(r),
                None if !(-0.05..=nominal + 0.05).contains(&d) => Some(Resync::FromEnd),
                None => None,
            };
            match resync {
                Some(Resync::FromEnd) => self.next_step = Self::first_step_from(b1 - origin, swing),
                Some(Resync::FromStart) => self.next_step = Self::first_step_from(b0 - origin, swing),
                None => {}
            }
            if resync != Some(Resync::FromEnd) && d > 0.0 {
                let bpf = d / n as f64;
                loop {
                    let t = step_time(self.next_step, self.patterns[self.current].swing) + origin;
                    if t >= b1 {
                        break;
                    }
                    if t >= b0 - CATCH_UP_BEATS {
                        let at = (((t - b0) / bpf - 1e-6).ceil().max(0.0) as usize).min(n - 1);
                        self.render_voices(done, at);
                        done = at;
                        self.fire_step(self.next_step);
                    }
                    self.next_step += 1;
                }
            }
        }
        self.render_voices(done, n);
    }

    /// Drum controls; `false` if `c` is not one the engine handles.
    pub(crate) fn control(&mut self, c: Control, v: ControlValue) -> bool {
        let press = matches!(v, ControlValue::Press(true));
        let abs = match v {
            ControlValue::Absolute(x) => Some(x.clamp(0.0, 1.0)),
            _ => None,
        };
        let steps = match v {
            ControlValue::Delta(d) => d.round() as i64,
            _ => 0,
        };
        let idx = |n: u8, max: usize| usize::from(n).checked_sub(1).filter(|&i| i < max);
        // Toggle on press, set on an absolute value.
        let switch = |on: bool| match v {
            ControlValue::Press(true) => Some(!on),
            ControlValue::Absolute(x) => Some(x >= 0.5),
            _ => None,
        };
        let sel = self.selected;
        match c {
            Control::DrumPlay => {
                if press {
                    self.playing = !self.playing;
                    if self.playing {
                        self.resync = Some(Resync::FromStart);
                        self.origin_request = true;
                        self.skip = [i64::MIN; INSTRUMENTS];
                    } else if let Some(q) = self.queued.take() {
                        self.current = q;
                        self.edited();
                    }
                }
            }
            Control::DrumRecord => {
                if press {
                    self.record = !self.record;
                }
            }
            Control::DrumStep(n) | Control::DrumAccent(n) | Control::DrumCell(n) | Control::DrumCellAccent(n) => {
                let (inst, step) = match c {
                    Control::DrumCell(_) | Control::DrumCellAccent(_) => match idx(n, CELLS) {
                        Some(i) => (i / STEPS, i % STEPS),
                        None => return true,
                    },
                    _ => match idx(n, STEPS) {
                        Some(s) => (sel, s),
                        None => return true,
                    },
                };
                let accent = matches!(c, Control::DrumAccent(_) | Control::DrumCellAccent(_));
                let pat = &mut self.patterns[self.current];
                if accent {
                    if let Some(on) = switch(pat.is_accent(inst, step)) {
                        pat.set_accent(inst, step, on);
                        self.edited();
                    }
                } else if let Some(on) = switch(pat.is_on(inst, step)) {
                    pat.set(inst, step, on);
                    self.edited();
                }
            }
            Control::DrumInst(n) => {
                if let (true, Some(i)) = (press, idx(n, INSTRUMENTS)) {
                    self.selected = i;
                    self.edited();
                }
            }
            Control::DrumTrigger(n) => {
                if let (true, Some(i)) = (press, idx(n, INSTRUMENTS)) {
                    self.trigger(i);
                }
            }
            Control::DrumInstSelect => {
                if steps != 0 {
                    self.selected = (self.selected as i64 + steps).rem_euclid(INSTRUMENTS as i64) as usize;
                    self.edited();
                }
            }
            Control::DrumInstMute(n) => {
                if let (true, Some(i)) = (press, idx(n, INSTRUMENTS)) {
                    self.muted[i] = !self.muted[i];
                    self.edited();
                }
            }
            Control::DrumInstLevel(n) | Control::DrumInstTune(n) | Control::DrumInstDecay(n) => {
                if let (Some(x), Some(i)) = (abs, idx(n, INSTRUMENTS)) {
                    self.set_knob(c, i, x);
                }
            }
            Control::DrumSelLevel | Control::DrumSelTune | Control::DrumSelDecay => {
                if let Some(x) = abs {
                    self.set_knob(c, sel, x);
                }
            }
            Control::DrumPattern(n) => {
                if let (true, Some(i)) = (press, idx(n, PATTERNS)) {
                    self.select_pattern(i);
                }
            }
            Control::DrumPatternSelect => {
                if steps != 0 {
                    let from = self.queued.unwrap_or(self.current) as i64;
                    self.select_pattern((from + steps).clamp(0, PATTERNS as i64 - 1) as usize);
                }
            }
            Control::DrumLength => {
                if steps != 0 {
                    let pat = &mut self.patterns[self.current];
                    pat.length = (i64::from(pat.length) + steps).clamp(1, STEPS as i64) as u8;
                    self.edited();
                }
            }
            Control::DrumSwing => {
                if let Some(x) = abs {
                    self.patterns[self.current].swing = x;
                    self.edited();
                }
            }
            Control::DrumClear => {
                if press {
                    self.patterns[self.current].clear_row(sel);
                    self.edited();
                }
            }
            Control::DrumClearPattern => {
                if press {
                    let length = self.patterns[self.current].length;
                    self.patterns[self.current] = Pattern { length, ..Pattern::default() };
                    self.edited();
                }
            }
            Control::DrumLevel => {
                if let Some(x) = abs {
                    self.strip.volume = x;
                    self.edited();
                }
            }
            Control::DrumFilter => {
                if let Some(x) = abs {
                    self.strip.filter = x;
                    self.edited();
                }
            }
            Control::DrumFxAssign(n) => {
                if let (true, Some(i)) = (press, idx(n, 2)) {
                    self.strip.fx_assign[i] = !self.strip.fx_assign[i];
                    self.edited();
                }
            }
            Control::DrumPfl => {
                if press {
                    self.strip.pfl = !self.strip.pfl;
                }
            }
            // Read-only, or handled by the app.
            Control::DrumStepLed(_)
            | Control::DrumInstLed(_)
            | Control::DrumMeter
            | Control::DrumKitSelect
            | Control::DrumShow => {}
            _ => return false,
        }
        true
    }

    fn set_knob(&mut self, c: Control, inst: usize, x: f32) {
        match c {
            Control::DrumInstLevel(_) | Control::DrumSelLevel => self.level[inst] = x,
            Control::DrumInstTune(_) | Control::DrumSelTune => self.tune[inst] = x,
            _ => self.decay[inst] = x,
        }
        self.edited();
    }

    /// While playing, the pattern starts when the current one comes round.
    fn select_pattern(&mut self, i: usize) {
        if self.playing && i != self.current {
            self.queued = Some(i);
        } else {
            self.queued = None;
            self.current = i;
            self.edited();
        }
    }

    pub(crate) fn state(&self, clock_beat: f64, origin: f64) -> crate::snapshot::DrumState {
        let pat = &self.patterns[self.current];
        let step = self.playing.then(|| {
            let s = ((clock_beat - origin) * 4.0).floor() as i64;
            pattern_step(s, pat.length()) as u8
        });
        crate::snapshot::DrumState {
            playing: self.playing,
            record: self.record,
            step,
            current: self.current as u8,
            queued: self.queued.map(|q| q as u8),
            selected: self.selected as u8,
            patterns: self.patterns,
            inst: std::array::from_fn(|i| crate::snapshot::DrumInstState {
                level: self.level[i],
                tune: self.tune[i],
                decay: self.decay[i],
                muted: self.muted[i],
                loaded: self.kit.as_ref().is_some_and(|k| k.samples[i].is_some()),
                hits: self.hits[i],
            }),
            channel: crate::snapshot::ChannelState {
                gain: self.strip.gain,
                eq: self.strip.eq,
                kill: self.strip.kill,
                filter: self.strip.filter,
                volume: self.strip.volume,
                pfl: self.strip.pfl,
                fx_assign: self.strip.fx_assign,
                meter: self.strip.meter.peak(),
            },
            edit_rev: self.edit_rev,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;
    const N: usize = 256;

    /// A kit whose samples are a single full-scale frame followed by
    /// silence (so the output shows exactly where each hit starts), with
    /// instrument `i` at level `0.1 * (i + 1)`.
    fn click_kit() -> Arc<DrumKit> {
        Arc::new(DrumKit {
            samples: std::array::from_fn(|i| {
                let mut frames = vec![[0.0f32; 2]; 2400];
                frames[0] = [0.1 * (i + 1) as f32; 2];
                Some(Arc::new(TrackAudio { sample_rate: SR, frames }))
            }),
        })
    }

    fn machine(pattern: Pattern) -> DrumMachine {
        let mut m = DrumMachine::new(SR, N);
        m.set_kit(Some(click_kit()));
        m.patterns = [Pattern::default(); PATTERNS];
        m.patterns[0] = pattern;
        m
    }

    fn row(inst: usize, text: &str) -> Pattern {
        let mut p = Pattern::default();
        p.set_row_text(inst, text);
        p
    }

    /// Plays from clock beat `start` at 120 BPM for `blocks` blocks; returns
    /// the left channel. `clock` maps the nominal beat to the clock's beat
    /// (to inject jumps).
    fn play(m: &mut DrumMachine, start: f64, blocks: usize, clock: impl Fn(usize, f64) -> f64) -> Vec<f32> {
        let bpf = 2.0 / f64::from(SR);
        m.last_end = clock(0, start);
        m.control(Control::DrumPlay, ControlValue::Press(true));
        let mut out = Vec::new();
        for k in 0..blocks {
            let end = clock(k + 1, start + bpf * ((k + 1) * N) as f64);
            m.render(N, end, 0.0, 120.0);
            out.extend(m.buf[..N].iter().map(|f| f[0]));
        }
        out
    }

    /// Frames where a hit starts, with its level.
    fn hits(out: &[f32]) -> Vec<(usize, f32)> {
        out.iter().enumerate().filter(|(_, v)| v.abs() > 1e-6).map(|(i, v)| (i, *v)).collect()
    }

    // At 120 BPM a sixteenth is 6000 frames.
    const SIXTEENTH: usize = 6000;

    #[test]
    fn steps_hit_on_the_exact_frame() {
        let mut m = machine(row(0, "x.x."));
        let out = play(&mut m, 0.0, 120, |_, b| b);
        let h = hits(&out);
        assert_eq!(h[0].0, 0, "step 1 at once: {h:?}");
        assert_eq!(h[1].0, 2 * SIXTEENTH);
        assert!((h[0].1 - 0.1 * NORMAL_GAIN).abs() < 1e-6, "normal gain");
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn accents_play_louder() {
        let mut m = machine(row(0, "X.x."));
        let h = hits(&play(&mut m, 0.0, 60, |_, b| b));
        assert!((h[0].1 - 0.1).abs() < 1e-6 && (h[1].1 - 0.07).abs() < 1e-6, "{h:?}");
    }

    #[test]
    fn swing_delays_off_beats() {
        let mut p = row(0, "xx..");
        p.swing = 1.0;
        let mut m = machine(p);
        let h = hits(&play(&mut m, 0.0, 60, |_, b| b));
        // Half a sixteenth late.
        assert_eq!(h[1].0, SIXTEENTH + SIXTEENTH / 2);
    }

    #[test]
    fn starting_mid_bar_plays_in_phase() {
        let mut m = machine(row(0, "x...x..."));
        // Start half a beat after the origin: step 5 (beat 1) comes next.
        let out = play(&mut m, 0.5, 60, |_, b| b);
        assert_eq!(hits(&out)[0].0, 2 * SIXTEENTH);
    }

    #[test]
    fn clock_jumps_resync_without_bursts() {
        let mut m = machine(row(0, "xxxxxxxxxxxxxxxx"));
        // A jump 0.45 beats ahead after 50 blocks, one back after 100.
        let out = play(&mut m, 0.0, 200, |k, b| {
            if k >= 100 {
                b + 0.45 - 0.3
            } else if k >= 50 {
                b + 0.45
            } else {
                b
            }
        });
        let h = hits(&out);
        for w in h.windows(2) {
            assert!(w[1].0 - w[0].0 > 200, "no hits squeezed together: {h:?}");
        }
    }

    #[test]
    fn backward_jitter_never_refires() {
        let mut m = machine(row(0, "x...x...x...x..."));
        // The clock wobbles back by a few frames every other block.
        let wobble = 0.0005;
        let out = play(&mut m, 0.0, 300, |k, b| if k % 2 == 1 { b - wobble } else { b });
        let h = hits(&out);
        // 300 blocks = 76800 frames = 3.2 beats: one hit per beat.
        assert_eq!(h.len(), 4, "{h:?}");
    }

    #[test]
    fn short_patterns_wrap() {
        let mut p = row(0, "x");
        p.length = 3;
        let mut m = machine(p);
        let h = hits(&play(&mut m, 0.0, 240, |_, b| b));
        let at: Vec<usize> = h.iter().map(|h| h.0).collect();
        assert_eq!(&at[..4], &[0, 3 * SIXTEENTH, 6 * SIXTEENTH, 9 * SIXTEENTH]);
    }

    #[test]
    fn queued_pattern_starts_when_the_current_comes_round() {
        let mut m = machine(row(0, "x...x...x...x..."));
        m.patterns[1] = row(1, "x...............");
        let bpf = 2.0 / f64::from(SR);
        m.control(Control::DrumPlay, ControlValue::Press(true));
        let mut out = Vec::new();
        for k in 0..400 {
            if k == 10 {
                m.control(Control::DrumPattern(2), ControlValue::Press(true));
                assert_eq!(m.queued, Some(1));
            }
            m.render(N, bpf * ((k + 1) * N) as f64, 0.0, 120.0);
            out.extend(m.buf[..N].iter().map(|f| f[0]));
        }
        let h = hits(&out);
        // Bar 1: four BD hits; from bar 2: SD on the one only.
        assert_eq!(h.iter().filter(|h| h.0 < 16 * SIXTEENTH).count(), 4);
        let bar2: Vec<_> = h.iter().filter(|h| h.0 >= 16 * SIXTEENTH && h.0 < 32 * SIXTEENTH).collect();
        assert_eq!(bar2.len(), 1);
        assert!((bar2[0].1 - 0.2 * NORMAL_GAIN).abs() < 1e-6);
        assert_eq!(m.current, 1);
    }

    #[test]
    fn closed_hat_chokes_open_hat() {
        let mut m = DrumMachine::new(SR, N);
        let long = Arc::new(TrackAudio { sample_rate: SR, frames: vec![[0.5; 2]; 48_000] });
        m.set_kit(Some(Arc::new(DrumKit {
            samples: std::array::from_fn(|i| (i == CHOKE.0 || i == CHOKE.1).then(|| long.clone())),
        })));
        m.control(Control::DrumTrigger(CHOKE.1 as u8 + 1), ControlValue::Press(true));
        m.render(N, 0.0, 0.0, 120.0);
        assert!(m.voices[CHOKE.1].iter().any(|v| v.active));
        m.control(Control::DrumTrigger(CHOKE.0 as u8 + 1), ControlValue::Press(true));
        for _ in 0..4 {
            m.render(N, 0.0, 0.0, 120.0);
        }
        assert!(m.voices[CHOKE.1].iter().all(|v| !v.active), "open hat cut");
        assert!(m.voices[CHOKE.0].iter().any(|v| v.active));
    }

    #[test]
    fn stopped_machine_plays_only_triggers() {
        let mut m = machine(row(0, "xxxxxxxxxxxxxxxx"));
        let bpf = 2.0 / f64::from(SR);
        for k in 0..100 {
            m.render(N, bpf * ((k + 1) * N) as f64, 0.0, 120.0);
            assert!(m.buf[..N].iter().all(|f| f[0] == 0.0));
        }
        m.control(Control::DrumTrigger(3), ControlValue::Press(true));
        m.render(N, bpf * (101 * N) as f64, 0.0, 120.0);
        assert!((m.buf[0][0] - 0.3).abs() < 1e-6, "trigger at full level: {}", m.buf[0][0]);
    }

    #[test]
    fn recording_writes_the_nearest_step_without_a_flam() {
        let mut m = machine(Pattern::default());
        m.control(Control::DrumRecord, ControlValue::Press(true));
        let bpf = 2.0 / f64::from(SR);
        m.control(Control::DrumPlay, ControlValue::Press(true));
        let mut out = Vec::new();
        for k in 0..100 {
            // Just before step 5 (beat 1 = 24000 frames).
            if k == 90 {
                m.control(Control::DrumTrigger(2), ControlValue::Press(true));
            }
            m.render(N, bpf * ((k + 1) * N) as f64, 0.0, 120.0);
            out.extend(m.buf[..N].iter().map(|f| f[0]));
        }
        assert!(m.patterns[0].is_on(1, 4), "step 5 written");
        assert_eq!(hits(&out).len(), 1, "played live once, not again on the step");
    }

    #[test]
    fn kit_change_mid_hit_is_safe() {
        let mut m = DrumMachine::new(SR, N);
        let long = Arc::new(TrackAudio { sample_rate: SR, frames: vec![[0.5; 2]; 48_000] });
        m.set_kit(Some(Arc::new(DrumKit { samples: std::array::from_fn(|_| Some(long.clone())) })));
        m.control(Control::DrumTrigger(1), ControlValue::Press(true));
        m.render(N, 0.0, 0.0, 120.0);
        // The new kit's sample is much shorter than where the hit is.
        let short = Arc::new(TrackAudio { sample_rate: SR, frames: vec![[0.5; 2]; 10] });
        let dropped = m.set_kit(Some(Arc::new(DrumKit { samples: std::array::from_fn(|_| Some(short.clone())) })));
        assert!(dropped.iter().all(Option::is_none), "the old kit waits for its hit to fade");
        m.render(N, 0.0, 0.0, 120.0);
        let peak = m.buf[..N].iter().fold(0.0f32, |p, f| p.max(f[0].abs()));
        assert!(peak <= 0.5 && m.buf[N - 1][0] == 0.0, "faded out");
        assert!(m.take_faded_kit().is_some());
    }

    #[test]
    fn tune_and_decay() {
        assert_eq!(decay_mul(1.0, 48_000.0), 1.0);
        assert!(decay_mul(0.0, 48_000.0) < decay_mul(0.5, 48_000.0));
        let mut m = DrumMachine::new(SR, N);
        let ramp =
            Arc::new(TrackAudio { sample_rate: SR, frames: (0..4800).map(|i| [i as f32 / 4800.0; 2]).collect() });
        m.set_kit(Some(Arc::new(DrumKit { samples: std::array::from_fn(|_| Some(ramp.clone())) })));
        m.control(Control::DrumInstTune(1), ControlValue::Absolute(1.0));
        m.control(Control::DrumTrigger(1), ControlValue::Press(true));
        m.render(N, 0.0, 0.0, 120.0);
        // An octave up reads two frames per frame.
        assert!((m.buf[100][0] - 200.0 / 4800.0).abs() < 1e-4, "{}", m.buf[100][0]);
    }

    #[test]
    fn controls() {
        let mut m = DrumMachine::new(SR, N);
        m.patterns[0] = Pattern::default();
        assert!(m.control(Control::DrumStep(3), ControlValue::Press(true)));
        assert!(m.patterns[0].is_on(0, 2));
        m.control(Control::DrumInst(2), ControlValue::Press(true));
        m.control(Control::DrumAccent(3), ControlValue::Press(true));
        assert!(m.patterns[0].is_accent(1, 2));
        m.control(Control::DrumCell(16 * 7 + 16), ControlValue::Absolute(1.0));
        assert!(m.patterns[0].is_on(7, 15));
        m.control(Control::DrumCell(16 * 7 + 16), ControlValue::Absolute(1.0));
        assert!(m.patterns[0].is_on(7, 15), "absolute sets, it does not toggle");
        m.control(Control::DrumInstSelect, ControlValue::Delta(-3.0));
        assert_eq!(m.selected, 6);
        m.control(Control::DrumLength, ControlValue::Delta(-20.0));
        assert_eq!(m.patterns[0].length, 1);
        m.control(Control::DrumPatternSelect, ControlValue::Delta(2.0));
        assert_eq!(m.current, 2, "stopped: switches at once");
        let rev = m.edit_rev;
        m.control(Control::DrumSelLevel, ControlValue::Absolute(0.3));
        assert_eq!(m.level[6], 0.3);
        assert_ne!(m.edit_rev, rev);
        assert!(!m.control(Control::Play, ControlValue::Press(true)));
    }
}
