//! A full channel + master chain driven through the public API with random
//! automation: no allocations, no NaN/Inf, and the limiter holds the ceiling.

use assert_no_alloc::assert_no_alloc;
use rille_dsp::fx::{EFFECT_NAMES, SLOTS};
use rille_dsp::{DjFilter, FxCtx, FxUnit, IsolatorEq, PeakLimiter, PeakMeter, SincTable, db_to_gain};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn channel_and_master_chain() {
    const SR: f32 = 48_000.0;
    rille_dsp::flush_denormals();
    let mut rng = Rng(0x1234_5678_9abc_def1);
    // A "track": noise bursts on the beat plus a bass tone, read at varispeed.
    let track: Vec<[f32; 2]> = (0..SR as usize * 4)
        .map(|i| {
            let t = i as f32 / SR;
            let kick = if (t * 2.0).fract() < 0.05 { rng.f32() * 2.0 - 1.0 } else { 0.0 };
            let v = 0.6 * (t * 55.0 * std::f32::consts::TAU).sin() + kick;
            [v, v * 0.9]
        })
        .collect();
    let sinc = SincTable::new(32, 256);
    let mut eq = IsolatorEq::new(SR);
    let mut filter = DjFilter::new(SR);
    let mut fx = FxUnit::new(SR);
    let mut limiter = PeakLimiter::new(SR);
    let mut meter = PeakMeter::new(SR);
    let ceiling = db_to_gain(-0.3);

    let mut buf = [[0.0f32; 2]; 128];
    let (mut pos, mut beat) = (0.0f64, 0.0f64);
    for block in 0..3000 {
        if block % 7 == 0 {
            match rng.below(6) {
                0 => eq.set_knobs(rng.f32(), rng.f32(), rng.f32()),
                1 => filter.set_knob(rng.f32()),
                2 => fx.select_effect(rng.below(SLOTS), rng.below(EFFECT_NAMES.len())),
                3 => fx.set_knob(rng.below(SLOTS), rng.below(3), rng.f32()),
                4 => fx.set_button(rng.below(SLOTS), rng.below(3), rng.below(2) == 0),
                _ => fx.set_on(rng.below(3) != 0),
            }
            fx.set_dry_wet(rng.f32());
        }
        let ratio = 0.5 + 1.5 * (block as f64 * 0.01).sin().abs();
        let ctx = FxCtx { sample_rate: SR, bpm: 120.0 * ratio, beat_pos: beat, beats_per_bar: 4 };
        assert_no_alloc(|| {
            let cutoff = SincTable::cutoff_for_ratio(ratio);
            for f in buf.iter_mut() {
                // Gain of 4 drives the limiter hard.
                let s = sinc.sample(&track, pos, cutoff);
                *f = [s[0] * 4.0, s[1] * 4.0];
                pos = (pos + ratio) % track.len() as f64;
            }
            eq.process(&mut buf);
            filter.process(&mut buf);
            fx.process(&mut buf, &ctx);
            limiter.process(&mut buf);
            meter.process(&buf);
        });
        for f in &buf {
            assert!(f[0].is_finite() && f[1].is_finite());
            assert!(f[0].abs() <= ceiling && f[1].abs() <= ceiling);
        }
        beat = ctx.beat_at(buf.len());
    }
    assert!(meter.hold()[0] > 0.5 && meter.hold()[0] <= ceiling);
}
