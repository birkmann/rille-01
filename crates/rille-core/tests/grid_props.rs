//! Property tests for the beatgrid contract the engine relies on.

use proptest::prelude::*;
use rille_core::beatgrid::{BeatClock, BeatMap, LiveMap, PiecewiseMap, TempoMarker};
use rille_core::quantize::{BeatLoop, phase_preserving_target, wrap_phase_error};

fn constant_map() -> impl Strategy<Value = BeatMap> {
    (-5.0f64..5.0, 40.0f64..240.0).prop_map(|(a, bpm)| BeatMap::constant(a, bpm).unwrap())
}

fn piecewise_map() -> impl Strategy<Value = BeatMap> {
    (-8.0f64..8.0, prop::collection::vec((1.0f64..60.0, 60.0f64..200.0), 1..6)).prop_map(|(first, parts)| {
        let mut secs = 0.0;
        let markers = parts
            .into_iter()
            .map(|(gap, bpm)| {
                let m = TempoMarker { secs, bpm };
                secs += gap;
                m
            })
            .collect();
        BeatMap::Piecewise(PiecewiseMap::new(first, markers).unwrap())
    })
}

fn live_map() -> impl Strategy<Value = BeatMap> {
    (0.0f64..2.0, prop::collection::vec(0.25f64..1.5, 1..400)).prop_map(|(start, gaps)| {
        let mut t = start;
        let mut beats = vec![t];
        for g in gaps {
            t += g;
            beats.push(t);
        }
        BeatMap::Live(LiveMap::new(beats).unwrap())
    })
}

fn any_map() -> impl Strategy<Value = BeatMap> {
    prop_oneof![constant_map(), piecewise_map(), live_map()]
}

proptest! {
    #[test]
    fn secs_beat_roundtrip(map in any_map(), secs in -30.0f64..900.0) {
        let back = map.secs_at(map.beat_at(secs));
        prop_assert!((back - secs).abs() < 1e-9, "{secs} -> {back}");
    }

    #[test]
    fn beat_secs_roundtrip(map in any_map(), beat in -64.0f64..2000.0) {
        let back = map.beat_at(map.secs_at(beat));
        prop_assert!((back - beat).abs() < 1e-9, "{beat} -> {back}");
    }

    #[test]
    fn strictly_monotone(map in any_map(), a in -30.0f64..900.0, d in 1e-6f64..10.0) {
        prop_assert!(map.beat_at(a + d) > map.beat_at(a));
        prop_assert!(map.bpm_at(a) > 0.0);
    }

    #[test]
    fn continuous(map in any_map(), s in -30.0f64..900.0) {
        // No jumps: beat position changes by at most bpm/60 * dt (+ rounding).
        let dt = 1e-7;
        let db = map.beat_at(s + dt) - map.beat_at(s);
        prop_assert!(db < 240.0 / 60.0 * dt * 4.0 + 1e-9);
    }

    #[test]
    fn phase_jump_preserves_phase(now in -100.0f64..1000.0, target in -100.0f64..1000.0) {
        let t = phase_preserving_target(now, target);
        prop_assert!(wrap_phase_error(t - now).abs() < 1e-9);
        prop_assert!((t - target).abs() <= 0.5 + 1e-9);
    }

    /// Wrapping a loop 1000+ times while advancing in small blocks must land
    /// where the analytic position says. The only error allowed is f64
    /// rounding of the millions of per-block additions (which can share a
    /// rounding direction), far below one sample: 1 sample at 48 kHz and
    /// 200 bpm is ~7e-5 beats, the bound is 1e-6.
    #[test]
    fn loop_wrap_has_no_drift(
        start in 0.0f64..500.0,
        len_idx in 0usize..11,
        step in 0.001f64..0.37,
    ) {
        let len = [1.0 / 32.0, 1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0][len_idx];
        let lp = BeatLoop { start_beat: start, len_beats: len };
        let steps = (len * 1000.0 / step).ceil() as u64;
        let mut pos = start;
        for _ in 0..steps {
            pos = lp.wrap(pos + step);
        }
        let expected = start + (steps as f64 * step).rem_euclid(len);
        let d = (pos - expected).abs();
        let err = d.min(len - d);
        prop_assert!(err < 1e-6, "err {err} after {steps} blocks");
    }
}
