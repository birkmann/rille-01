//! Level meter for VU displays.

use crate::denormal::undenormal;

const HOLD_SECS: f32 = 1.0;
const DECAY_DB_PER_SEC: f32 = 20.0;
const RMS_MS: f32 = 300.0;

/// Stereo peak/RMS meter. Feed it the audio blocks and read the levels
/// (linear, per channel) from the UI side, e.g. via a copy published once
/// per block.
#[derive(Clone, Debug)]
pub struct PeakMeter {
    sample_rate: f32,
    peak: [f32; 2],
    hold: [f32; 2],
    hold_age: [f32; 2],
    ms: [f32; 2],
    rms_coef: f32,
}

impl PeakMeter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            peak: [0.0; 2],
            hold: [0.0; 2],
            hold_age: [0.0; 2],
            ms: [0.0; 2],
            rms_coef: 1.0 - (-1.0 / (RMS_MS * 0.001 * sample_rate)).exp(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new(self.sample_rate);
    }

    pub fn process(&mut self, buf: &[[f32; 2]]) {
        if buf.is_empty() {
            return;
        }
        let dt = buf.len() as f32 / self.sample_rate;
        let decay = 10f32.powf(-DECAY_DB_PER_SEC * dt / 20.0);
        for ch in 0..2 {
            let mut block_peak = 0f32;
            let mut ms = self.ms[ch];
            for f in buf {
                let x = f[ch];
                // `max` ignores NaN, so a bad sample cannot poison the meter.
                block_peak = block_peak.max(x.abs());
                let sq = x * x;
                if sq.is_finite() {
                    ms += (sq - ms) * self.rms_coef;
                }
            }
            self.ms[ch] = undenormal(ms);
            self.peak[ch] = block_peak.max(undenormal(self.peak[ch] * decay));
            if block_peak >= self.hold[ch] {
                self.hold[ch] = block_peak;
                self.hold_age[ch] = 0.0;
            } else {
                self.hold_age[ch] += dt;
                if self.hold_age[ch] > HOLD_SECS {
                    self.hold[ch] = undenormal(self.hold[ch] * decay).max(self.peak[ch]);
                }
            }
        }
    }

    /// Peak level: jumps up instantly, falls at 20 dB/s.
    pub fn peak(&self) -> [f32; 2] {
        self.peak
    }

    /// RMS level over ~300 ms.
    pub fn rms(&self) -> [f32; 2] {
        [self.ms[0].sqrt(), self.ms[1].sqrt()]
    }

    /// Peak hold: the highest peak, held for 1 s, then falling at 20 dB/s.
    pub fn hold(&self) -> [f32; 2] {
        self.hold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::sine;

    const SR: f32 = 48_000.0;

    #[test]
    fn peak_hold_decay_and_rms() {
        let mut m = PeakMeter::new(SR);
        let s = sine(SR, 1000.0, 0.5, 96_000);
        for b in s.chunks(480) {
            m.process(b);
        }
        assert!((m.peak()[0] - 0.5).abs() < 1e-3);
        assert!((m.rms()[1] - 0.5 / 2f32.sqrt()).abs() < 5e-3, "{:?}", m.rms());
        // Silence: peak falls 20 dB per second, hold stays for a second first.
        let silence = [[0.0f32; 2]; 480];
        for _ in 0..50 {
            assert_no_alloc::assert_no_alloc(|| m.process(&silence));
        }
        assert!((crate::gain_to_db(m.peak()[0] / 0.5) + 10.0).abs() < 0.1);
        assert!((m.hold()[0] - 0.5).abs() < 1e-3);
        for _ in 0..100 {
            m.process(&silence);
        }
        let hold_db = crate::gain_to_db(m.hold()[0] / 0.5);
        assert!(hold_db < -9.0 && hold_db > -11.0, "{hold_db}");
        assert!(m.rms()[0] < 0.035); // 1.5 s at a 300 ms time constant
        m.process(&[[f32::NAN, 1.0]]);
        assert!(m.peak()[0].is_finite() && m.rms()[0].is_finite());
    }
}
