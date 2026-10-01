//! Brickwall peak limiter and soft clipper for the master output.

use crate::{db_to_gain, gain_to_db};

/// Default ceiling in dBFS.
pub const DEFAULT_CEILING_DB: f32 = -0.3;
const LOOKAHEAD_MS: f32 = 1.0;
const RELEASE_MS: f32 = 50.0;

/// Sliding-window minimum over the last `cap` values (monotonic deque in a
/// fixed ring).
#[derive(Clone, Debug)]
struct MinWindow {
    idx: Box<[u64]>,
    val: Box<[f32]>,
    head: usize,
    len: usize,
}

impl MinWindow {
    fn new(cap: usize) -> Self {
        Self { idx: vec![0; cap].into_boxed_slice(), val: vec![0.0; cap].into_boxed_slice(), head: 0, len: 0 }
    }

    /// Adds value `v` with index `i` (strictly increasing) and returns the
    /// minimum over indices `i + 1 - cap ..= i`.
    #[inline]
    fn push(&mut self, i: u64, v: f32) -> f32 {
        let cap = self.idx.len();
        while self.len > 0 && self.idx[self.head] + cap as u64 <= i {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
        while self.len > 0 && self.val[(self.head + self.len - 1) % cap] >= v {
            self.len -= 1;
        }
        let back = (self.head + self.len) % cap;
        self.idx[back] = i;
        self.val[back] = v;
        self.len += 1;
        self.val[self.head]
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// Stereo lookahead peak limiter with linked gain.
///
/// The gain needed by each frame is spread over the lookahead window
/// (sliding minimum, then a moving average of the same length), so the gain
/// has already ramped down when a peak leaves the delay line. Because every
/// averaged value is at most the gain required by the output frame, the
/// output never exceeds the ceiling; a final clamp only catches float
/// rounding.
#[derive(Clone, Debug)]
pub struct PeakLimiter {
    ceiling: f32,
    delay: Box<[[f32; 2]]>,
    delay_pos: usize,
    window: MinWindow,
    avg_buf: Box<[f32]>,
    avg_pos: usize,
    avg_sum: f64,
    env: f32,
    release: f32,
    gain: f32,
    frame: u64,
}

impl PeakLimiter {
    pub fn new(sample_rate: f32) -> Self {
        let lookahead = ((LOOKAHEAD_MS * 0.001 * sample_rate).round() as usize).max(1);
        let w = lookahead + 1;
        let mut l = Self {
            ceiling: db_to_gain(DEFAULT_CEILING_DB),
            delay: vec![[0.0; 2]; lookahead].into_boxed_slice(),
            delay_pos: 0,
            window: MinWindow::new(w),
            avg_buf: vec![1.0; w].into_boxed_slice(),
            avg_pos: 0,
            avg_sum: w as f64,
            env: 1.0,
            release: (-1.0 / (RELEASE_MS * 0.001 * sample_rate)).exp(),
            gain: 1.0,
            frame: 0,
        };
        l.reset();
        l
    }

    /// Sets the ceiling in dBFS (clamped to −24..=0).
    pub fn set_ceiling_db(&mut self, db: f32) {
        if db.is_finite() {
            self.ceiling = db_to_gain(db.clamp(-24.0, 0.0));
        }
    }

    pub fn ceiling_db(&self) -> f32 {
        gain_to_db(self.ceiling)
    }

    /// Delay added by the lookahead.
    pub fn latency_frames(&self) -> usize {
        self.delay.len()
    }

    /// Current gain reduction in dB (0 = none, positive = reducing).
    pub fn gain_reduction_db(&self) -> f32 {
        -gain_to_db(self.gain)
    }

    pub fn reset(&mut self) {
        self.delay.fill([0.0; 2]);
        self.delay_pos = 0;
        self.window.clear();
        self.avg_buf.fill(1.0);
        self.avg_pos = 0;
        self.avg_sum = self.avg_buf.len() as f64;
        self.env = 1.0;
        self.gain = 1.0;
        self.frame = 0;
    }

    /// One frame, before the safety clamp.
    #[inline]
    fn tick_raw(&mut self, x: [f32; 2]) -> [f32; 2] {
        let peak = x[0].abs().max(x[1].abs());
        let need = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
        self.frame += 1;
        let m = self.window.push(self.frame, need);
        // Instant attack, exponential release; never above `m`.
        self.env = if m < self.env { m } else { m + (self.env - m) * self.release };
        self.avg_sum += (self.env - self.avg_buf[self.avg_pos]) as f64;
        self.avg_buf[self.avg_pos] = self.env;
        self.avg_pos = (self.avg_pos + 1) % self.avg_buf.len();
        self.gain = ((self.avg_sum / self.avg_buf.len() as f64) as f32).min(1.0);
        let out = self.delay[self.delay_pos];
        self.delay[self.delay_pos] = x;
        self.delay_pos = (self.delay_pos + 1) % self.delay.len();
        [out[0] * self.gain, out[1] * self.gain]
    }

    #[inline]
    pub fn tick(&mut self, x: [f32; 2]) -> [f32; 2] {
        let y = self.tick_raw(x);
        let c = self.ceiling;
        [y[0].clamp(-c, c), y[1].clamp(-c, c)]
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        for f in buf.iter_mut() {
            *f = self.tick(*f);
        }
        // Resynchronise the running sum so rounding can never accumulate.
        self.avg_sum = self.avg_buf.iter().map(|&v| v as f64).sum();
    }
}

/// Stateless soft clipper for when the limiter is off: linear up to a knee,
/// then a tanh curve approaching the ceiling.
#[derive(Clone, Copy, Debug)]
pub struct SoftClip {
    ceiling: f32,
    knee: f32,
}

impl Default for SoftClip {
    fn default() -> Self {
        Self::new(DEFAULT_CEILING_DB)
    }
}

impl SoftClip {
    /// Knee as a fraction of the ceiling.
    const KNEE: f32 = 0.7;

    pub fn new(ceiling_db: f32) -> Self {
        let mut s = Self { ceiling: 1.0, knee: Self::KNEE };
        s.set_ceiling_db(ceiling_db);
        s
    }

    pub fn set_ceiling_db(&mut self, db: f32) {
        if db.is_finite() {
            self.ceiling = db_to_gain(db.clamp(-24.0, 6.0));
            self.knee = self.ceiling * Self::KNEE;
        }
    }

    /// Clips one sample: unity below `knee`, smooth approach to `ceiling`
    /// above, continuous in value and slope.
    #[inline]
    pub fn shape(x: f32, knee: f32, ceiling: f32) -> f32 {
        let a = x.abs();
        if a <= knee {
            return x;
        }
        let r = ceiling - knee;
        (knee + r * ((a - knee) / r).tanh()).copysign(x)
    }

    #[inline]
    pub fn tick(&self, x: f32) -> f32 {
        Self::shape(x, self.knee, self.ceiling)
    }

    pub fn process(&self, buf: &mut [[f32; 2]]) {
        for f in buf.iter_mut() {
            *f = [self.tick(f[0]), self.tick(f[1])];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Rng, all_finite, sine};

    const SR: f32 = 48_000.0;

    /// Runs `input` through a fresh limiter and checks the pre-clamp output
    /// against the ceiling (after the latency).
    fn check(input: &[[f32; 2]], ceiling_db: f32) -> Vec<[f32; 2]> {
        let mut l = PeakLimiter::new(SR);
        l.set_ceiling_db(ceiling_db);
        let c = l.ceiling;
        let mut out = Vec::with_capacity(input.len());
        for (i, &x) in input.iter().enumerate() {
            let y = l.tick_raw(x);
            assert!(y[0].abs() <= c * (1.0 + 1e-6) && y[1].abs() <= c * (1.0 + 1e-6), "frame {i}: {y:?} > {c}");
            out.push(y);
        }
        out
    }

    #[test]
    fn never_exceeds_ceiling_on_adversarial_input() {
        let n = 48_000;
        // Full-scale square waves at several periods, including single-frame alternation.
        for period in [2, 3, 7, 48, 97, 480] {
            let sq: Vec<[f32; 2]> =
                (0..n).map(|i| if (i / (period / 2).max(1)) % 2 == 0 { [1.0, -1.0] } else { [-1.0, 1.0] }).collect();
            check(&sq, DEFAULT_CEILING_DB);
        }
        // Isolated spikes over silence and over a quiet sine, one channel only.
        let mut rng = Rng::new(5);
        let mut spikes = sine(SR, 440.0, 0.1, n);
        for _ in 0..200 {
            let i = rng.below(n);
            spikes[i][rng.below(2)] = 30.0 * rng.bipolar();
        }
        check(&spikes, DEFAULT_CEILING_DB);
        // +20 dB sine and noise, with other ceilings.
        check(&sine(SR, 1234.0, 10.0, n), DEFAULT_CEILING_DB);
        let noise: Vec<[f32; 2]> = (0..n).map(|_| [8.0 * rng.bipolar(), 8.0 * rng.bipolar()]).collect();
        check(&noise, -6.0);
        check(&noise, 0.0);
    }

    #[test]
    fn transparent_below_ceiling_and_latency() {
        let mut l = PeakLimiter::new(SR);
        let lat = l.latency_frames();
        assert_eq!(lat, 48);
        let input = sine(SR, 1000.0, 0.5, 4800);
        let mut buf = input.clone();
        assert_no_alloc::assert_no_alloc(|| l.process(&mut buf));
        for i in lat..buf.len() {
            assert_eq!(buf[i], input[i - lat]);
        }
        assert_eq!(l.gain_reduction_db(), 0.0);
    }

    #[test]
    fn loud_sine_is_limited_and_releases() {
        let mut l = PeakLimiter::new(SR);
        let mut buf = sine(SR, 100.0, 4.0, 24_000);
        l.process(&mut buf);
        let peak = buf[12_000..].iter().flatten().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.9 && peak <= l.ceiling, "{peak}");
        let gr = l.gain_reduction_db();
        assert!(gr > 11.0 && gr < 13.0, "{gr}");
        // After 0.5 s of quiet signal the gain is back to (almost) unity.
        let mut quiet = sine(SR, 100.0, 0.1, 24_000);
        l.process(&mut quiet);
        assert!(l.gain_reduction_db() < 0.01);
        assert!(all_finite(&quiet));
    }

    #[test]
    fn soft_clip_is_bounded_and_continuous() {
        let s = SoftClip::default();
        let c = db_to_gain(DEFAULT_CEILING_DB);
        assert_eq!(s.tick(0.5), 0.5);
        assert!(s.tick(100.0) <= c && s.tick(-100.0) >= -c);
        let mut prev = s.tick(-3.0);
        for i in 1..6000 {
            let v = s.tick(-3.0 + i as f32 * 0.001);
            assert!(v >= prev && v - prev <= 0.001 + 1e-6);
            prev = v;
        }
        let mut buf = vec![[2.0f32, -0.1]; 4];
        s.process(&mut buf);
        assert!(buf[0][0] < c && buf[0][1] == -0.1);
    }
}
