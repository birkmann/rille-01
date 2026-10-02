//! A playback head: reads a track at a variable speed, either by sinc
//! interpolation (varispeed, scratching, reverse) or through a time-stretcher
//! (keylock, key shift). It always knows which source frame is audible now,
//! so sync and quantized jumps work identically on both paths.

use rille_dsp::{SincTable, cubic_hermite};
use signalsmith_stretch::Stretch;

use crate::types::{StemAudio, TrackAudio};

/// The audio a voice reads: a track, or a track with its stems at their own
/// levels ([`StemMix`]).
pub(crate) trait Source {
    fn len(&self) -> usize;
    /// Frame `i` (`< len`).
    fn frame(&self, i: usize) -> [f32; 2];
    /// A slice that holds frames `lo..hi` (clipped to the source), and the
    /// source index of the slice's first frame.
    fn window(&mut self, lo: usize, hi: usize) -> (&[[f32; 2]], usize);
}

impl Source for &TrackAudio {
    fn len(&self) -> usize {
        self.frames.len()
    }

    fn frame(&self, i: usize) -> [f32; 2] {
        self.frames[i]
    }

    fn window(&mut self, _lo: usize, _hi: usize) -> (&[[f32; 2]], usize) {
        (&self.frames, 0)
    }
}

/// A track with its stems at `gains` (drums, bass, other, vocals; 1 =
/// unchanged), mixed on the fly into `scratch` for the frames a block reads.
pub(crate) struct StemMix<'a> {
    pub track: &'a TrackAudio,
    pub stems: &'a StemAudio,
    pub gains: [f32; 4],
    pub scratch: &'a mut [[f32; 2]],
}

/// The mixed frame: the track at the other stem's gain, plus each stored
/// stem at the difference to it.
fn mix_frame(track: &TrackAudio, stems: &StemAudio, gains: &[f32; 4], i: usize) -> [f32; 2] {
    const SCALE: f32 = 1.0 / 32768.0;
    let (t, s) = (track.frames[i], stems.frames.get(i).copied().unwrap_or_default());
    let [drums, bass, other, vocals] = *gains;
    let (d, b, v) = ((drums - other) * SCALE, (bass - other) * SCALE, (vocals - other) * SCALE);
    let ch = |c: usize| t[c] * other + d * f32::from(s[c]) + b * f32::from(s[2 + c]) + v * f32::from(s[4 + c]);
    [ch(0), ch(1)]
}

impl Source for StemMix<'_> {
    fn len(&self) -> usize {
        self.track.frames.len()
    }

    fn frame(&self, i: usize) -> [f32; 2] {
        mix_frame(self.track, self.stems, &self.gains, i)
    }

    fn window(&mut self, lo: usize, hi: usize) -> (&[[f32; 2]], usize) {
        let hi = hi.min(self.len()).min(lo + self.scratch.len());
        let lo = lo.min(hi);
        for (k, out) in self.scratch[..hi - lo].iter_mut().enumerate() {
            *out = mix_frame(self.track, self.stems, &self.gains, lo + k);
        }
        (&self.scratch[..hi - lo], lo)
    }
}

/// Source frames the sinc interpolator reads on either side of a position
/// (at its lowest cutoff).
pub(crate) const READ_REACH: usize = 130;

pub(crate) struct Voice {
    /// Sinc path: source frame of the next output frame. Stretch path: next
    /// input frame to feed (always whole).
    pos: f64,
    stretch: Stretch,
    stretching: bool,
    /// Fractional input frames owed to the stretcher (keeps the average rate exact).
    carry: f64,
    lat_in: f64,
    lat_out: f64,
    /// Interleaved scratch buffers for the stretcher.
    input: Vec<f32>,
    output: Vec<f32>,
    preroll: usize,
    /// Source frames per output frame in the last render.
    last_speed: f64,
    /// Stretch path: (output frames, input frames) of the latest renders,
    /// newest at `hist_head`. The audio heard now went in `lat_out` output
    /// frames ago, at whatever speeds were used then; after a jump the
    /// stretcher was primed at `prime_speed`.
    hist: [(f64, f64); HIST],
    hist_len: usize,
    hist_head: usize,
    prime_speed: f64,
    /// Stretch path: the source repeats with this period (a looping sample
    /// read as one endless stream, so its wraps need no jump).
    wrap: Option<usize>,
    /// Output discarded while priming the stretcher after a jump.
    prime_out: Vec<f32>,
}

/// Renders remembered for the stretcher's latency (≥ its output latency).
const HIST: usize = 64;

/// Max source frames per output frame on the stretch path.
const MAX_STRETCH_SPEED: f64 = 4.0;

impl Voice {
    pub fn new(sample_rate: u32, max_block: usize) -> Self {
        let mut stretch = Stretch::preset_default(2, sample_rate);
        let preroll = 16_384;
        // The first seek/process allocate inside the C++ library; do it here.
        let warm = vec![0.0f32; 2 * preroll];
        stretch.seek(&warm, 1.0);
        let mut out = vec![0.0f32; 2 * max_block];
        stretch.process(&warm[..2 * max_block], &mut out);
        stretch.reset();
        let (lat_in, lat_out) = (stretch.input_latency() as f64, stretch.output_latency() as f64);
        // Priming needs about one output latency (half a block) of output.
        let prime = (lat_out * 1.25).ceil() as usize;
        Self {
            pos: 0.0,
            stretch,
            stretching: false,
            carry: 0.0,
            lat_in,
            lat_out,
            input: vec![0.0; 2 * ((max_block as f64 * MAX_STRETCH_SPEED) as usize + 64).max(preroll)],
            output: vec![0.0; 2 * max_block],
            preroll,
            last_speed: 1.0,
            hist: [(0.0, 0.0); HIST],
            hist_len: 0,
            hist_head: 0,
            prime_speed: 1.0,
            wrap: None,
            prime_out: vec![0.0; 2 * prime],
        }
    }

    /// Makes the stretch path read the source as a loop of `period` frames
    /// (`None`: once, silence outside). The sinc path ignores it.
    pub fn set_wrap(&mut self, period: Option<usize>) {
        self.wrap = period.filter(|&p| p > 0);
    }

    /// Source frame being heard now. On the stretch path the input advances
    /// in whole frames; adding the fractional carry keeps this continuous, so
    /// followers see a smooth tempo instead of ±1-frame steps.
    pub fn audible(&self) -> f64 {
        if self.stretching { self.pos + self.carry - self.lat_in - self.in_flight() } else { self.pos }
    }

    /// Input frames inside the stretcher's output latency: consumed during
    /// the last `lat_out` output frames (primed at `prime_speed` before the
    /// history starts).
    fn in_flight(&self) -> f64 {
        let mut out_left = self.lat_out;
        let mut input = 0.0;
        for k in 0..self.hist_len {
            let (o, i) = self.hist[(self.hist_head + HIST - k) % HIST];
            if o >= out_left {
                return input + i * out_left / o.max(1e-9);
            }
            out_left -= o;
            input += i;
        }
        input + out_left * self.prime_speed
    }

    fn record(&mut self, out: f64, input: f64) {
        self.hist_head = (self.hist_head + 1) % HIST;
        self.hist[self.hist_head] = (out, input);
        self.hist_len = (self.hist_len + 1).min(HIST);
    }

    pub fn is_stretching(&self) -> bool {
        self.stretching
    }

    /// Moves so that `target` (source frame) is audible from the next output
    /// frame on. On the stretch path the stretcher is primed with the audio
    /// before that point, so there is no latency gap: it is fed the input
    /// before the jump and runs for one output latency with the output
    /// discarded. A seek alone would leave its output fading in over ~50 ms,
    /// swallowing an attack right at the target (a hotcue on a kick).
    pub fn jump(&mut self, audio: &mut impl Source, target: f64, speed: f64, stretch: bool) {
        self.last_speed = speed;
        self.carry = 0.0;
        self.hist_len = 0;
        self.prime_speed = speed;
        if !stretch {
            self.stretching = false;
            self.pos = target;
            return;
        }
        self.stretching = true;
        let speed = speed.max(0.01);
        let end = (target + self.lat_in + self.lat_out * speed).round();
        let prime = self.prime_out.len() / 2;
        let primed_in = ((prime as f64 * speed).round() as usize).min(self.input.len() / 2);
        let pre = self.preroll;
        let start = end as i64 - primed_in as i64;
        read_frames(audio, start - pre as i64, &mut self.input[..2 * pre], self.wrap);
        // Drop whatever the stretcher still holds from its last use, or it
        // would play out after the new audio (a ghost of an old beat).
        self.stretch.reset();
        self.stretch.seek(&self.input[..2 * pre], speed);
        read_frames(audio, start, &mut self.input[..2 * primed_in], self.wrap);
        self.stretch.process(&self.input[..2 * primed_in], &mut self.prime_out);
        self.pos = end;
        self.record(prime as f64, primed_in as f64);
    }

    /// Renders `out.len()` frames. `speed` is source frames per output frame
    /// (negative = backwards); `transpose` the stretcher's pitch factor.
    /// `cubic` selects cheaper interpolation while scratching.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        audio: &mut impl Source,
        out: &mut [[f32; 2]],
        speed: f64,
        stretch: bool,
        transpose: f32,
        sinc: &SincTable,
        cubic: bool,
    ) {
        let n = out.len();
        if stretch && speed > 0.0 && speed <= MAX_STRETCH_SPEED {
            if !self.stretching {
                let a = self.audible();
                self.jump(audio, a, speed, true);
            }
            let want = speed * n as f64 + self.carry;
            let n_in = (want.floor() as usize).min(self.input.len() / 2);
            self.carry = want - n_in as f64;
            read_frames(audio, self.pos as i64, &mut self.input[..2 * n_in], self.wrap);
            self.stretch.set_transpose_factor(transpose, None);
            self.stretch.process(&self.input[..2 * n_in], &mut self.output[..2 * n]);
            for (o, s) in out.iter_mut().zip(self.output.chunks_exact(2)) {
                *o = [s[0], s[1]];
            }
            self.pos += n_in as f64;
            // Exact (fractional) input, as `pos + carry` advances.
            self.record(n as f64, speed * n as f64);
        } else {
            if self.stretching {
                self.pos = self.audible();
                self.stretching = false;
            }
            let cutoff = SincTable::cutoff_for_ratio(speed);
            // The frames this block reads, and where they start.
            let last = self.pos + speed * n.saturating_sub(1) as f64;
            let lo = (self.pos.min(last).floor() - READ_REACH as f64).max(0.0) as usize;
            let hi = (self.pos.max(last).ceil() + READ_REACH as f64).max(0.0) as usize + 1;
            let (frames, base) = audio.window(lo, hi);
            let base = base as f64;
            for (i, o) in out.iter_mut().enumerate() {
                let p = self.pos + speed * i as f64 - base;
                *o = if cubic { cubic_hermite(frames, p) } else { sinc.sample(frames, p, cutoff) };
            }
            self.pos += speed * n as f64;
        }
        self.last_speed = speed;
    }
}

/// Copies frames `[start, start + dst.len()/2)` interleaved; silence outside
/// the track, or the track repeated every `wrap` frames.
fn read_frames(audio: &impl Source, start: i64, dst: &mut [f32], wrap: Option<usize>) {
    let len = audio.len() as i64;
    for (i, d) in dst.chunks_exact_mut(2).enumerate() {
        let mut idx = start + i as i64;
        if let Some(w) = wrap {
            idx = idx.rem_euclid(w as i64);
        }
        let f = if idx >= 0 && idx < len { audio.frame(idx as usize) } else { [0.0, 0.0] };
        d[0] = f[0];
        d[1] = f[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretched_jump_keeps_the_attack() {
        // A click right at (or just after) the jump target comes out at full
        // level on its frame: the stretcher is primed, not faded in.
        const SR: u32 = 48_000;
        let mut frames = vec![[0.0f32; 2]; 96_000];
        frames.iter_mut().skip(24_000).take(240).for_each(|f| *f = [0.9, 0.9]);
        let audio = TrackAudio { sample_rate: SR, frames };
        let sinc = SincTable::new(32, 256);
        for back in [0usize, 48, 480, 4800] {
            let mut v = Voice::new(SR, 256);
            v.jump(&mut &audio, (24_000 - back) as f64, 1.0, true);
            assert!((v.audible() - (24_000 - back) as f64).abs() < 1.0, "{}", v.audible());
            let mut out = vec![[0.0f32; 2]; 256];
            let mut all = Vec::new();
            for _ in 0..30 {
                v.render(&mut &audio, &mut out, 1.0, true, 1.0, &sinc, false);
                all.extend(out.iter().map(|f| f[0]));
            }
            let onset = all.iter().position(|v| v.abs() > 0.5);
            assert_eq!(onset, Some(back), "jumped {back} frames before the click");
        }
    }

    #[test]
    fn wrapped_stream_repeats_the_source() {
        let audio = TrackAudio { sample_rate: 48_000, frames: (0..10).map(|i| [i as f32, 0.0]).collect() };
        let mut dst = vec![0.0f32; 2 * 25];
        read_frames(&&audio, -3, &mut dst, Some(10));
        let got: Vec<f32> = dst.chunks(2).map(|f| f[0]).collect();
        assert_eq!(&got[..5], &[7.0, 8.0, 9.0, 0.0, 1.0]);
        assert_eq!(got[24], 1.0);
        read_frames(&&audio, 8, &mut dst[..8], None);
        assert_eq!(dst[..8], [8.0, 0.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }
}
