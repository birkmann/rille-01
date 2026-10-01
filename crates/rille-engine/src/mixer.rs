//! Channel strip: gain, 3-band isolator EQ, filter, fader, headphone tap.

use rille_dsp::{DjFilter, FilterMode, IsolatorEq, LinearSmoother, PeakMeter, db_to_gain};

use crate::snapshot::ChannelState;
use crate::types::FX_UNITS;

pub(crate) struct Strip {
    /// Knob positions 0..1 (0.5 = neutral for gain, EQ and filter).
    pub gain: f32,
    pub eq: [f32; 3],
    /// EQ kills held (low, mid, high): the band as with its knob at 0.
    pub kill: [bool; 3],
    pub filter: f32,
    pub volume: f32,
    pub pfl: bool,
    pub fx_assign: [bool; FX_UNITS],
    iso: IsolatorEq,
    dj_filter: DjFilter,
    pre_gain: LinearSmoother,
    post_gain: LinearSmoother,
    pub meter: PeakMeter,
}

/// GAIN knob: ±12 dB around the centre.
pub fn gain_knob_db(v: f32) -> f32 {
    (v - 0.5) * 24.0
}

/// Channel fader curve: quadratic, unity at the top.
pub fn fader_gain(v: f32) -> f32 {
    v.clamp(0.0, 1.0).powi(2)
}

/// Crossfader gain for the left (`side = 0`) or right side. Each side stays
/// at full level until the fader passes a point set by `curve`, then falls
/// off with a quarter cosine: from its own end (`curve` 0, a constant-power
/// fade), the middle (0.5, both at full level there) or just before the far
/// end (1, a scratch cut).
pub fn crossfader_gain(x: f32, side: usize, curve: f32) -> f32 {
    let x = if side == 0 { x } else { 1.0 - x };
    let c = curve.clamp(0.0, 1.0);
    let start = if c <= 0.5 { c } else { 0.5 + (c - 0.5) * 0.94 };
    if x <= start { 1.0 } else { ((x - start) / (1.0 - start) * std::f32::consts::FRAC_PI_2).cos().max(0.0) }
}

/// EQ knob positions with held kills at 0.
fn eq_knobs(eq: [f32; 3], kill: [bool; 3]) -> [f32; 3] {
    std::array::from_fn(|k| if kill[k] { 0.0 } else { eq[k] })
}

/// What a channel does to the waveform's low, mid and high bands (split at
/// 250 Hz and 3 kHz), as linear amplitude gains: GAIN knob, EQ and filter.
/// A display estimate taken at a typical frequency of each band.
pub fn band_gains(ch: &ChannelState) -> [f32; 3] {
    const BAND_HZ: [f32; 3] = [80.0, 900.0, 7000.0];
    let gain = db_to_gain(gain_knob_db(ch.gain));
    let filter = DjFilter::cutoff_for_knob(ch.filter);
    let eq = eq_knobs(ch.eq, ch.kill);
    std::array::from_fn(|k| {
        let f = BAND_HZ[k];
        // 12 dB/octave, like the channel filter.
        let filtered = match filter {
            None => 1.0,
            Some((FilterMode::LowPass, fc)) => 1.0 / (1.0 + (f / fc).powi(4)).sqrt(),
            Some((FilterMode::HighPass, fc)) => 1.0 / (1.0 + (fc / f).powi(4)).sqrt(),
        };
        gain * rille_dsp::eq::knob_to_gain(eq[k]) * filtered
    })
}

/// Level after the channel fader and the crossfader, given the channel's
/// crossfader gain (see [`crate::Snapshot::crossfader_gain`]).
pub fn fader_level(ch: &ChannelState, crossfader_gain: f32) -> f32 {
    fader_gain(ch.volume) * crossfader_gain
}

impl Strip {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            gain: 0.5,
            eq: [0.5; 3],
            kill: [false; 3],
            filter: 0.5,
            volume: 1.0,
            pfl: false,
            fx_assign: [false; FX_UNITS],
            iso: IsolatorEq::new(sample_rate),
            dj_filter: DjFilter::new(sample_rate),
            pre_gain: LinearSmoother::new(sample_rate, 20.0, 1.0),
            post_gain: LinearSmoother::new(sample_rate, 20.0, 1.0),
            meter: PeakMeter::new(sample_rate),
        }
    }

    /// Gain, EQ and filter in place (the pre-fader signal), then metering:
    /// like a hardware mixer's channel meter, it shows a playing track with
    /// the fader down.
    pub fn process_pre(&mut self, buf: &mut [[f32; 2]], auto_gain_db: f32) {
        self.pre_gain.set_target(db_to_gain(gain_knob_db(self.gain) + auto_gain_db));
        for f in buf.iter_mut() {
            let g = self.pre_gain.tick();
            f[0] *= g;
            f[1] *= g;
        }
        let [lo, mid, hi] = eq_knobs(self.eq, self.kill);
        self.iso.set_knobs(lo, mid, hi);
        self.iso.process(buf);
        self.dj_filter.set_knob(self.filter);
        self.dj_filter.process(buf);
        self.meter.process(buf);
    }

    /// Fader and crossfader gain in place.
    pub fn process_post(&mut self, buf: &mut [[f32; 2]], xfade: f32) {
        self.post_gain.set_target(fader_gain(self.volume) * xfade);
        for f in buf.iter_mut() {
            let g = self.post_gain.tick();
            f[0] *= g;
            f[1] *= g;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves() {
        assert_eq!(crossfader_gain(0.5, 0, 0.5), 1.0);
        assert_eq!(crossfader_gain(0.5, 1, 0.5), 1.0);
        assert!(crossfader_gain(1.0, 0, 0.5) < 1e-6);
        assert_eq!(crossfader_gain(0.0, 0, 0.5), 1.0);
        assert!(crossfader_gain(0.0, 1, 0.5) < 1e-6);
        assert!((crossfader_gain(0.75, 0, 0.5) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        // Slow fade: constant power, both sides at -3 dB in the middle.
        let (l, r) = (crossfader_gain(0.5, 0, 0.0), crossfader_gain(0.5, 1, 0.0));
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && (l - r).abs() < 1e-6);
        assert!(crossfader_gain(0.3, 0, 0.0) < 1.0);
        // Scratch cut: full level until the last few percent.
        assert_eq!(crossfader_gain(0.96, 0, 1.0), 1.0);
        assert!(crossfader_gain(1.0, 0, 1.0) < 1e-6);
        for c in [0.0, 0.3, 0.5, 0.8, 1.0] {
            let g: Vec<f32> = (0..=100).map(|i| crossfader_gain(i as f32 / 100.0, 0, c)).collect();
            assert!(g.windows(2).all(|w| w[1] <= w[0] + 1e-6), "monotonic at curve {c}");
        }
        assert_eq!(fader_gain(1.0), 1.0);
        assert_eq!(gain_knob_db(0.5), 0.0);
    }

    #[test]
    fn band_gains_follow_the_knobs() {
        let neutral = ChannelState { gain: 0.5, eq: [0.5; 3], filter: 0.5, volume: 1.0, ..Default::default() };
        assert_eq!(band_gains(&neutral), [1.0; 3]);
        let low_kill = ChannelState { eq: [0.0, 0.5, 0.5], ..neutral };
        assert_eq!(band_gains(&low_kill), [0.0, 1.0, 1.0]);
        let held_kill = ChannelState { kill: [false, false, true], ..neutral };
        assert_eq!(band_gains(&held_kill), [1.0, 1.0, 0.0]);
        let [lo, mid, hi] = band_gains(&ChannelState { gain: 1.0, ..neutral });
        assert!((lo - 3.981).abs() < 0.01 && lo == mid && mid == hi, "+12 dB on all bands");
        // Low-pass most of the way: highs gone, lows kept.
        let [lo, _, hi] = band_gains(&ChannelState { filter: 0.1, ..neutral });
        assert!(lo > 0.9 && hi < 0.01, "{lo} {hi}");
        let [lo, _, hi] = band_gains(&ChannelState { filter: 0.9, ..neutral });
        assert!(lo < 0.1 && hi > 0.9, "{lo} {hi}");
        assert_eq!(fader_level(&ChannelState { volume: 0.0, ..neutral }, 1.0), 0.0);
        assert_eq!(fader_level(&ChannelState { volume: 0.5, ..neutral }, 0.5), 0.125);
    }

    #[test]
    fn meter_is_pre_fader() {
        let mut strip = Strip::new(48_000.0);
        strip.volume = 0.0;
        let mut buf = vec![[0.5f32, 0.5]; 4800];
        strip.process_pre(&mut buf, 0.0);
        strip.process_post(&mut buf, 1.0);
        assert!(buf.last().unwrap()[0].abs() < 1e-6, "fader down mutes the channel");
        assert!(strip.meter.peak()[0] > 0.4, "{:?}", strip.meter.peak());
    }
}
