//! Remix deck: four slots of sixteen cells, each cell a loop or one-shot
//! sample, played in time with the deck.
//!
//! The deck itself runs on a virtual timeline with a constant grid at the
//! set's tempo, so sync, master and phase alignment treat it like a track
//! deck. Every slot derives its sample position from the deck's beat each
//! block: a loop started at beat `s` plays sample beat `(b - s) mod len` at
//! deck beat `b`. Triggers wait for the next quantize boundary and start on
//! the exact frame; loop wraps and jumps are crossfaded like a track deck's.

use std::sync::Arc;

use rille_core::remix::{CELLS, PAGES, ROWS, SLOTS, pad_cell};
use rille_core::{BeatGrid, BeatMap, Control, ControlValue, GridSource};
use rille_dsp::{DjFilter, LinearSmoother, PeakMeter};

use crate::deck::RenderCtx;
use crate::mixer::fader_gain;
use crate::types::{MAX_DECKS, TrackAudio};
use crate::voice::Voice;

/// Quantize sizes in beats, selectable with `RemixQuantizeSize`.
pub const REMIX_QUANT_SIZES: [f64; 6] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
/// One bar.
pub const DEFAULT_REMIX_QUANT: usize = 4;

/// A sample in a remix cell.
#[derive(Clone)]
pub struct RemixSample {
    pub audio: Arc<TrackAudio>,
    /// The sample's tempo; it always plays at the deck's tempo.
    pub bpm: f64,
    /// Loops repeat; one-shots play once and stop the slot.
    pub looped: bool,
    /// Index into `rille_core::remix::COLORS`.
    pub color: u8,
}

impl RemixSample {
    /// Length in beats at the sample's tempo.
    pub fn beats(&self) -> f64 {
        self.audio.duration_secs() * self.bpm / 60.0
    }

    /// Source frames per output frame at `bpf` deck beats per output frame.
    fn speed(&self, bpf: f64) -> f64 {
        bpf * 60.0 / self.bpm * f64::from(self.audio.sample_rate)
    }
}

/// Plays one cell at a time through two voices (crossfaded jumps).
struct SlotPlayer {
    voices: [Voice; 2],
    /// Cell each voice reads.
    voice_cell: [Option<usize>; 2],
    active: usize,
    fade_pos: usize,
    fade_len: usize,
    /// The new voice starts at full level (a sample's first frame: fading
    /// it in would soften its attack); only the old one fades out.
    hard_in: bool,
    buf: Vec<[f32; 2]>,
    fade_buf: Vec<[f32; 2]>,
    /// Cell playing, and the deck beat its sample beat 0 sits on.
    cell: Option<usize>,
    start_beat: f64,
    /// Cell waiting to start at a deck beat.
    pending: Option<(usize, f64)>,
    volume: f32,
    filter: f32,
    muted: bool,
    dj_filter: DjFilter,
    gain: LinearSmoother,
    meter: PeakMeter,
}

impl SlotPlayer {
    fn new(sample_rate: u32, max_block: usize) -> Self {
        let sr = sample_rate as f32;
        Self {
            voices: [Voice::new(sample_rate, max_block), Voice::new(sample_rate, max_block)],
            voice_cell: [None; 2],
            active: 0,
            fade_pos: 0,
            fade_len: 0,
            hard_in: false,
            buf: vec![[0.0; 2]; max_block],
            fade_buf: vec![[0.0; 2]; max_block],
            cell: None,
            start_beat: 0.0,
            pending: None,
            volume: 1.0,
            filter: 0.5,
            muted: false,
            dj_filter: DjFilter::new(sr),
            gain: LinearSmoother::new(sr, 10.0, 1.0),
            meter: PeakMeter::new(sr),
        }
    }

    /// Stops at once, fading out whatever sounds.
    fn stop(&mut self, declick: usize) {
        self.cell = None;
        self.pending = None;
        if self.voice_cell[self.active].is_some() {
            self.active = 1 - self.active;
            self.voice_cell[self.active] = None;
            self.fade_pos = 0;
            self.fade_len = declick;
            self.hard_in = false;
        }
    }

    /// Starts `cell` at deck beat `at` (the beat its sample starts on).
    /// Switching between loops keeps the phase: the new loop continues
    /// where the old one's bar would be.
    fn start(&mut self, cells: &[Option<RemixSample>], cell: usize, at: f64) {
        let Some(sample) = &cells[cell] else { return };
        let from_loop = self.cell.is_some_and(|c| c != cell && cells[c].as_ref().is_some_and(|s| s.looped));
        if !(sample.looped && from_loop) {
            self.start_beat = at;
        }
        self.cell = Some(cell);
    }

    /// Moves the voice for `cell` to source frame `target`, crossfading from
    /// whatever plays now.
    fn jump(&mut self, sample: &RemixSample, cell: usize, target: f64, bpf: f64, stretch: bool, declick: usize) {
        let speed = sample.speed(bpf).max(0.01);
        if self.voice_cell[self.active].is_some() && declick > 0 {
            self.active = 1 - self.active;
            self.fade_pos = 0;
            self.fade_len = declick;
            self.hard_in = target < 1.0;
        } else {
            self.fade_len = 0;
        }
        let voice = &mut self.voices[self.active];
        voice.set_wrap(sample.looped.then_some(sample.audio.frames.len()));
        voice.jump(&mut &*sample.audio, target, speed, stretch);
        self.voice_cell[self.active] = Some(cell);
    }

    /// Renders `n` frames from deck beat `b0`, advancing `bpf` beats per
    /// output frame, into `self.buf`.
    fn render(&mut self, cells: &[Option<RemixSample>], n: usize, b0: f64, bpf: f64, stretch: bool, ctx: &RenderCtx) {
        self.buf[..n].fill([0.0; 2]);
        if bpf <= 0.0 {
            return;
        }
        let mut done = 0;
        while done < n {
            let beat = b0 + bpf * done as f64;
            if let Some((cell, at)) = self.pending
                && at <= beat + 1e-9
            {
                self.pending = None;
                self.start(cells, cell, at);
            }
            let mut seg = n - done;
            if let Some((_, at)) = self.pending {
                seg = seg.min((((at - beat) / bpf).ceil() as usize).max(1));
            }
            if let Some(c) = self.cell {
                let Some(sample) = &cells[c] else {
                    // Emptied while playing.
                    self.stop(0);
                    continue;
                };
                let len = sample.beats();
                let rel = beat - self.start_beat;
                let pos = if sample.looped && len > 0.0 { rel.rem_euclid(len) } else { rel };
                if !sample.looped && (pos >= len || pos < 0.0) {
                    // One-shot played out (its audio has ended too).
                    self.stop(0);
                    continue;
                }
                // The sinc path jumps back to the start at a loop's end; the
                // stretch path reads loops as an endless stream (a jump
                // there costs the stretcher's priming).
                let voice = &self.voices[self.active];
                let cyclic = sample.looped && stretch && voice.is_stretching();
                if sample.looped && !cyclic {
                    seg = seg.min((((len - pos) / bpf).ceil() as usize).max(1));
                }
                let sr = f64::from(sample.audio.sample_rate);
                let target = pos * 60.0 / sample.bpm * sr;
                let drift = if cyclic {
                    let period = sample.audio.frames.len() as f64;
                    let d = (voice.audible() - target).rem_euclid(period);
                    d.min(period - d)
                } else {
                    (voice.audible() - target).abs()
                };
                if self.voice_cell[self.active] != Some(c) || drift > 0.001 * sr {
                    self.jump(sample, c, target, bpf, stretch, ctx.declick);
                }
            }
            if self.voice_cell.iter().any(Option::is_some) {
                self.render_voices(cells, done, seg, bpf, stretch, ctx);
            }
            done += seg;
        }
    }

    fn render_voices(
        &mut self,
        cells: &[Option<RemixSample>],
        from: usize,
        n: usize,
        bpf: f64,
        stretch: bool,
        ctx: &RenderCtx,
    ) {
        let render = |voice: &mut Voice, cell: Option<usize>, out: &mut [[f32; 2]], stretch: bool| match cell
            .and_then(|c| cells[c].as_ref())
        {
            Some(s) => {
                let transpose = (f64::from(s.audio.sample_rate) / ctx.sr_out) as f32;
                voice.render(&mut &*s.audio, out, s.speed(bpf), stretch, transpose, ctx.sinc, false);
            }
            None => out.fill([0.0; 2]),
        };
        let out = &mut self.buf[from..from + n];
        render(&mut self.voices[self.active], self.voice_cell[self.active], out, stretch);
        if self.fade_pos < self.fade_len {
            let old = 1 - self.active;
            let fade = &mut self.fade_buf[..n];
            let old_stretch = stretch && self.voices[old].is_stretching();
            render(&mut self.voices[old], self.voice_cell[old], fade, old_stretch);
            for (i, (o, f)) in out.iter_mut().zip(fade.iter()).enumerate() {
                let g = ((self.fade_pos + i) as f32 / self.fade_len as f32).min(1.0);
                let (gi, go) = ((g * std::f32::consts::FRAC_PI_2).sin(), (g * std::f32::consts::FRAC_PI_2).cos());
                let gi = if self.hard_in { 1.0 } else { gi };
                o[0] = o[0] * gi + f[0] * go;
                o[1] = o[1] * gi + f[1] * go;
            }
            self.fade_pos += n;
            if self.fade_pos >= self.fade_len {
                self.voice_cell[old] = None;
            }
        }
    }

    /// Where the playing cell is, `0..1` of its length.
    fn progress(&self, cells: &[Option<RemixSample>], beat: f64) -> f32 {
        let Some(s) = self.cell.and_then(|c| cells[c].as_ref()) else { return 0.0 };
        let len = s.beats();
        if len <= 0.0 {
            return 0.0;
        }
        let rel = (beat - self.start_beat).max(0.0);
        let pos = if s.looped { rel.rem_euclid(len) } else { rel.min(len) };
        (pos / len) as f32
    }
}

/// The remix half of a deck. Built on the app side (it allocates) and moved
/// into the engine with `Command::SetRemix`.
pub struct RemixDeck {
    pub(crate) id: u64,
    /// Moved into the deck when installed.
    pub(crate) grid: Option<Arc<BeatGrid>>,
    pub(crate) cells: Vec<Option<RemixSample>>,
    slots: Vec<SlotPlayer>,
    pub(crate) page: usize,
    pub(crate) quantize: bool,
    pub(crate) quant: usize,
    pub(crate) capture_source: u8,
    /// Virtual play position, seconds on the deck's grid.
    pub(crate) pos: f64,
}

impl RemixDeck {
    /// A remix deck for deck `deck` at `bpm`; `id` identifies it like a
    /// loaded track's id (for `Command::SetGrid`).
    pub fn new(sample_rate: u32, max_block: usize, deck: u8, id: u64, bpm: f64) -> Box<Self> {
        Box::new(Self {
            id,
            grid: Some(Arc::new(remix_grid(bpm))),
            cells: vec![None; CELLS],
            slots: (0..SLOTS).map(|_| SlotPlayer::new(sample_rate, max_block)).collect(),
            page: 0,
            quantize: true,
            quant: DEFAULT_REMIX_QUANT,
            capture_source: default_capture_source(deck),
            pos: 0.0,
        })
    }

    pub(crate) fn stop_all(&mut self, declick: usize) {
        for s in &mut self.slots {
            s.stop(declick);
        }
    }

    /// Queues `cell`: at `beat` when `now`, else at the next quantize
    /// boundary (or at once without quantize). Empty cells do nothing.
    pub(crate) fn trigger(&mut self, cell: usize, beat: f64, now: bool) {
        if self.cells.get(cell).is_none_or(Option::is_none) {
            return;
        }
        let at = if now || !self.quantize {
            beat
        } else {
            let q = REMIX_QUANT_SIZES[self.quant];
            ((beat / q + 1e-9).floor() + 1.0) * q
        };
        self.slots[cell / ROWS].pending = Some((cell, at));
    }

    /// Remix controls; `false` if `c` is not one (the deck handles it).
    pub(crate) fn control(&mut self, c: Control, v: ControlValue, deck: u8, declick: usize) -> bool {
        let press = matches!(v, ControlValue::Press(true));
        let abs = match v {
            ControlValue::Absolute(x) => Some(x.clamp(0.0, 1.0)),
            _ => None,
        };
        // Encoders send one step per tick; the UI may jump several.
        let steps = match v {
            ControlValue::Delta(d) => d.round() as i64,
            _ => 0,
        };
        let step = steps.signum();
        let slot = |n: u8| usize::from(n).checked_sub(1).filter(|&s| s < SLOTS);
        match c {
            Control::RemixStop(n) => {
                if let (true, Some(s)) = (press, slot(n)) {
                    self.slots[s].stop(declick);
                }
            }
            Control::RemixMute(n) => {
                if let (true, Some(s)) = (press, slot(n)) {
                    self.slots[s].muted = !self.slots[s].muted;
                }
            }
            Control::RemixVolume(n) => {
                if let (Some(x), Some(s)) = (abs, slot(n)) {
                    self.slots[s].volume = x;
                }
            }
            Control::RemixFilter(n) => {
                if let (Some(x), Some(s)) = (abs, slot(n)) {
                    self.slots[s].filter = x;
                }
            }
            Control::RemixPage => self.page = (self.page as i64 + steps).clamp(0, PAGES as i64 - 1) as usize,
            Control::RemixQuantize if press => self.quantize = !self.quantize,
            Control::RemixQuantizeSize => {
                self.quant = (self.quant as i64 + steps).clamp(0, REMIX_QUANT_SIZES.len() as i64 - 1) as usize;
            }
            Control::RemixCaptureSource if step != 0 => {
                // Cycle through the other decks.
                let n = MAX_DECKS as i64;
                let mut d = i64::from(self.capture_source);
                loop {
                    d = (d + step).rem_euclid(n);
                    if d != i64::from(deck) {
                        break;
                    }
                }
                self.capture_source = d as u8;
            }
            // Editing cells (delete, capture, type, load) is the app's job.
            Control::RemixQuantize
            | Control::RemixCaptureSource
            | Control::RemixPadDelete(_)
            | Control::RemixPadCapture(_)
            | Control::RemixPadType(_)
            | Control::RemixPadLoad(_) => {}
            _ => return false,
        }
        true
    }

    /// Cell of pad `pad` (1..=16) on the current page.
    pub(crate) fn pad_cell(&self, pad: u8) -> Option<usize> {
        pad_cell(pad, self.page)
    }

    /// Renders `n` frames into `out`, the deck moving from beat `b0` by `bpf`
    /// beats per output frame (0 when stopped).
    pub(crate) fn render(&mut self, out: &mut [[f32; 2]], b0: f64, bpf: f64, stretch: bool, ctx: &RenderCtx) {
        let n = out.len();
        out.fill([0.0; 2]);
        for slot in &mut self.slots {
            slot.render(&self.cells, n, b0, bpf, stretch, ctx);
            let buf = &mut slot.buf[..n];
            slot.dj_filter.set_knob(slot.filter);
            slot.dj_filter.process(buf);
            slot.gain.set_target(if slot.muted { 0.0 } else { fader_gain(slot.volume) });
            for (o, s) in out.iter_mut().zip(buf.iter_mut()) {
                let g = slot.gain.tick();
                s[0] *= g;
                s[1] *= g;
                o[0] += s[0];
                o[1] += s[1];
            }
            slot.meter.process(buf);
        }
    }

    pub(crate) fn state(&self, beat: f64) -> crate::snapshot::RemixState {
        let mut st = crate::snapshot::RemixState {
            active: true,
            page: self.page as u8,
            quantize: self.quantize,
            quant_idx: self.quant as u8,
            capture_source: self.capture_source,
            ..Default::default()
        };
        for (c, cell) in self.cells.iter().enumerate() {
            if let Some(s) = cell {
                st.cells[c] = crate::snapshot::RemixCellState { loaded: true, looped: s.looped, color: s.color };
            }
        }
        for (i, s) in self.slots.iter().enumerate() {
            st.slots[i] = crate::snapshot::RemixSlotState {
                cell: s.cell.map(|c| c as u8),
                queued: s.pending.map(|(c, _)| c as u8),
                progress: s.progress(&self.cells, beat),
                volume: s.volume,
                filter: s.filter,
                muted: s.muted,
                meter: s.meter.peak(),
            };
        }
        st
    }
}

/// The deck's own timeline: constant tempo, bar 1 at 0 s.
pub fn remix_grid(bpm: f64) -> BeatGrid {
    let bpm = if bpm.is_finite() && bpm > 0.0 { bpm.clamp(40.0, 250.0) } else { 120.0 };
    BeatGrid::new(BeatMap::constant(0.0, bpm).expect("valid tempo"), GridSource::Manual)
}

/// CAPTURE takes from the track deck on the same side: C ← A, D ← B, and
/// A and B from each other.
pub fn default_capture_source(deck: u8) -> u8 {
    match deck {
        0 => 1,
        1 => 0,
        d => d - 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rille_dsp::SincTable;

    const SR: u32 = 48_000;

    /// A sample whose frame `i` has value `i` (so the output shows exactly
    /// which source frame plays), `beats` long at 120 BPM.
    fn ramp(beats: f64, looped: bool) -> RemixSample {
        let frames = (beats * 0.5 * f64::from(SR)) as usize;
        let audio = TrackAudio { sample_rate: SR, frames: (0..frames).map(|i| [i as f32, 0.0]).collect() };
        RemixSample { audio: Arc::new(audio), bpm: 120.0, looped, color: 1 }
    }

    fn ctx(sinc: &SincTable) -> RenderCtx<'_> {
        RenderCtx { sr_out: f64::from(SR), sinc, declick: 0 }
    }

    /// Renders slot 0 alone (no filter or gain), `blocks` blocks of `n`
    /// frames at 120 BPM from beat `b0`.
    fn play(r: &mut RemixDeck, b0: f64, blocks: usize, n: usize) -> Vec<f32> {
        let sinc = SincTable::new(32, 256);
        let bpf = 2.0 / f64::from(SR);
        let mut out = Vec::new();
        for k in 0..blocks {
            let slot = &mut r.slots[0];
            slot.render(&r.cells, n, b0 + bpf * (k * n) as f64, bpf, false, &ctx(&sinc));
            out.extend(slot.buf[..n].iter().map(|f| f[0]));
        }
        out
    }

    #[test]
    fn quantized_trigger_starts_on_the_boundary_frame() {
        let mut r = RemixDeck::new(SR, 512, 2, 1, 120.0);
        r.cells[0] = Some(ramp(4.0, true));
        r.quant = 2; // 1 beat
        r.trigger(0, 0.3, false);
        assert_eq!(r.slots[0].pending.map(|p| p.1), Some(1.0));
        // Beat 1 is 0.7 beats = 16800 frames after 0.3.
        let out = play(&mut r, 0.3, 40, 512);
        assert!(out[..16800].iter().all(|&v| v == 0.0));
        assert!(out[16800].abs() < 1.0, "first frame of the sample: {}", out[16800]);
        assert!((out[16900] - 100.0).abs() < 1.0, "{}", out[16900]);
        assert!((out[20000] - 3200.0).abs() < 1.0, "{}", out[20000]);
    }

    #[test]
    fn loops_wrap_and_one_shots_stop() {
        let mut r = RemixDeck::new(SR, 512, 2, 1, 120.0);
        r.cells[0] = Some(ramp(1.0, true)); // 24000 frames
        r.trigger(0, 0.0, true);
        let out = play(&mut r, 0.0, 100, 512);
        assert!((out[23999] - 23999.0).abs() < 1.0);
        assert!(out[24000].abs() < 1.0, "wrapped: {}", out[24000]);
        assert!((out[24000 + 777] - 777.0).abs() < 1.0);

        let mut r = RemixDeck::new(SR, 512, 2, 1, 120.0);
        r.cells[0] = Some(ramp(1.0, false));
        r.trigger(0, 0.0, true);
        let out = play(&mut r, 0.0, 100, 512);
        assert!((out[23000] - 23000.0).abs() < 1.0);
        assert!(out[25000..].iter().all(|&v| v == 0.0));
        assert_eq!(r.slots[0].cell, None);
    }

    #[test]
    fn plays_at_the_deck_tempo_and_switches_loops_in_phase() {
        let mut r = RemixDeck::new(SR, 512, 2, 1, 120.0);
        // A 60 BPM sample: one beat = 48000 frames, played twice as fast.
        let mut slow = ramp(2.0, true);
        slow.bpm = 60.0;
        r.cells[0] = Some(slow);
        r.cells[1] = Some(ramp(4.0, true));
        r.trigger(0, 0.0, true);
        let out = play(&mut r, 0.0, 30, 512);
        assert!((out[10000] - 20000.0).abs() < 2.0, "{}", out[10000]);
        // Switch to cell 1 at beat 1 (24000 frames): it continues at its beat 1.
        r.quant = 2;
        r.trigger(1, 15360.0 * 2.0 / f64::from(SR), false);
        let b = 15360.0 * 2.0 / f64::from(SR);
        let out = play(&mut r, b, 30, 512);
        let at = 24000 - 15360;
        assert!((out[at + 10] - (24000.0 + 10.0)).abs() < 2.0, "{}", out[at + 10]);
    }

    #[test]
    fn stretched_loops_keep_every_beat() {
        // A one-bar loop with a 5 ms click on each beat, through the
        // stretcher (keylock) with crossfades on: no beat may be softened,
        // the first one (the trigger) and every wrap included.
        let mut r = RemixDeck::new(SR, 256, 2, 1, 120.0);
        let mut frames = vec![[0.0f32; 2]; 96_000];
        for b in 0..4 {
            frames.iter_mut().skip(b * 24_000).take(240).for_each(|f| *f = [0.9, 0.9]);
        }
        let audio = Arc::new(TrackAudio { sample_rate: SR, frames });
        r.cells[0] = Some(RemixSample { audio, bpm: 120.0, looped: true, color: 1 });
        r.trigger(0, 0.0, true);
        let sinc = SincTable::new(32, 256);
        let ctx = RenderCtx { sr_out: f64::from(SR), sinc: &sinc, declick: 144 };
        let bpf = 2.0 / f64::from(SR);
        let mut out = Vec::new();
        for k in 0..800 {
            r.slots[0].render(&r.cells, 256, bpf * (k * 256) as f64, bpf, true, &ctx);
            out.extend(r.slots[0].buf[..256].iter().map(|f| f[0]));
        }
        for b in 0..8 {
            let peak = out[b * 24_000..b * 24_000 + 600].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.8, "beat {b}: {peak}");
        }
    }

    #[test]
    fn controls() {
        let mut r = RemixDeck::new(SR, 64, 2, 1, 120.0);
        assert_eq!(r.capture_source, 0);
        assert!(r.control(Control::RemixPage, ControlValue::Delta(1.0), 2, 0));
        assert_eq!(r.pad_cell(1), Some(4));
        r.control(Control::RemixPage, ControlValue::Delta(2.0), 2, 0);
        assert_eq!(r.page, 3, "the UI jumps several pages at once");
        r.control(Control::RemixPage, ControlValue::Delta(9.0), 2, 0);
        assert_eq!(r.page, PAGES - 1);
        r.control(Control::RemixQuantizeSize, ControlValue::Delta(-3.0), 2, 0);
        assert_eq!(REMIX_QUANT_SIZES[r.quant], 0.5);
        r.control(Control::RemixCaptureSource, ControlValue::Delta(1.0), 2, 0);
        assert_eq!(r.capture_source, 1);
        r.control(Control::RemixCaptureSource, ControlValue::Delta(1.0), 2, 0);
        assert_eq!(r.capture_source, 3, "skips the deck itself");
        r.control(Control::RemixVolume(2), ControlValue::Absolute(0.25), 2, 0);
        assert_eq!(r.slots[1].volume, 0.25);
        assert!(!r.control(Control::Play, ControlValue::Press(true), 2, 0));
        // Empty cells ignore triggers.
        r.trigger(5, 0.0, true);
        assert!(r.slots[0].pending.is_none());
    }
}
