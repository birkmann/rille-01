//! Beatgrid accuracy on synthetic tracks with exactly known beats.
//!
//! Run in release for speed: `cargo test --release -p rille-analysis`.

use rille_analysis::synth::{self, Spec};
use rille_analysis::{AnalysisConfig, analyze};
use rille_core::beatgrid::BeatMap;
use rille_core::{BeatClock, BeatGrid, Key};

struct Eval {
    grid: BeatGrid,
    /// Max |grid − truth| over all beats (ms).
    max_err_ms: f64,
    p95_ms: f64,
    /// Spread of the error (max − min), i.e. error without a constant offset.
    spread_ms: f64,
    /// Truth beat 0 (a downbeat) lands on a grid downbeat.
    downbeat_ok: bool,
    /// Grid has as many beats as the truth between first and last beat.
    count_ok: bool,
}

fn eval(spec: &Spec, truth_pulse: bool) -> Eval {
    let r = synth::render(spec);
    let out = analyze(&r.audio, &AnalysisConfig::default());
    let grid = out.analysis.grid.expect("grid detected");
    let truth = if truth_pulse { &r.pulse } else { &r.beats };
    let errs: Vec<f64> = truth
        .iter()
        .map(|&t| {
            let b = grid.beat_at(t).round();
            (grid.secs_at(b) - t) * 1000.0
        })
        .collect();
    let max_err_ms = errs.iter().fold(0.0f64, |m, e| m.max(e.abs()));
    let spread_ms = errs.iter().cloned().fold(f64::MIN, f64::max) - errs.iter().cloned().fold(f64::MAX, f64::min);
    let mut abs: Vec<f64> = errs.iter().map(|e| e.abs()).collect();
    abs.sort_by(f64::total_cmp);
    let p95_ms = abs[abs.len() * 95 / 100];
    let worst: Vec<(usize, f64)> = {
        let mut v: Vec<(usize, f64)> = errs.iter().copied().enumerate().collect();
        v.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        v.into_iter().take(5).collect()
    };
    println!("worst beats (index, ms): {worst:?} of {}", errs.len());
    let b0 = grid.beat_at(truth[0]).round() as i64;
    let b_last = grid.beat_at(*truth.last().unwrap()).round() as i64;
    Eval {
        downbeat_ok: grid.is_downbeat(b0),
        count_ok: (b_last - b0) as usize == truth.len() - 1,
        grid,
        max_err_ms,
        p95_ms,
        spread_ms,
    }
}

fn bpm_of(grid: &BeatGrid) -> f64 {
    match &grid.map {
        BeatMap::Constant(m) => m.bpm(),
        _ => panic!("expected a constant grid, got {:?}", grid.map),
    }
}

#[test]
fn constant_tempo_is_exact() {
    let cases = [
        (128.0, 44_100, 0.5, 1),
        (124.0, 48_000, 1.237, 2),
        (174.0, 44_100, 0.1, 3),
        (95.0, 44_100, 2.0, 4),
        (140.0, 96_000, 0.77, 5),
        (122.5, 44_100, 0.333, 6),
        (126.37, 44_100, 0.9, 7),
        (133.33, 48_000, 0.01, 8),
    ];
    for (bpm, sr, first, seed) in cases {
        let spec = Spec { sr, sections: vec![(80, bpm)], first_beat_secs: first, seed, ..Spec::default() };
        let e = eval(&spec, false);
        let got = bpm_of(&e.grid);
        println!(
            "bpm {bpm:7.2} sr {sr}: got {got:.4}  max err {:.3} ms  spread {:.3} ms  conf {:.2}  flags {:?}",
            e.max_err_ms, e.spread_ms, e.grid.confidence, e.grid.flags
        );
        assert!((got - bpm).abs() < 0.005, "bpm {bpm}: got {got}");
        assert!(e.max_err_ms < 1.0, "bpm {bpm}: max err {} ms", e.max_err_ms);
        assert!(e.count_ok, "bpm {bpm}: beat count mismatch (octave error)");
        assert!(e.downbeat_ok, "bpm {bpm}: wrong downbeat");
        assert!(e.grid.confidence > 0.8, "bpm {bpm}: confidence {}", e.grid.confidence);
    }
}

/// A loud rolling sub bass between the kicks stacks more kick-band flux
/// than the kick itself; the grid must still sit on the kick.
#[test]
fn rolling_bass_does_not_pull_grid_off_the_kick() {
    for (bpm, seed) in [(128.0, 51), (136.0, 52), (122.0, 53)] {
        let spec = Spec { sections: vec![(80, bpm)], rolling_bass: true, seed, ..Spec::default() };
        let e = eval(&spec, false);
        println!("rolling bass {bpm}: max err {:.3} ms, flags {:?}", e.max_err_ms, e.grid.flags);
        assert!((bpm_of(&e.grid) - bpm).abs() < 0.005, "bpm {bpm}: got {}", bpm_of(&e.grid));
        assert!(e.max_err_ms < 1.5, "bpm {bpm}: max err {} ms", e.max_err_ms);
        assert!(e.count_ok, "bpm {bpm}: beat count mismatch");
    }
}

#[test]
fn breakdown_without_kick_keeps_grid() {
    let spec = Spec { sections: vec![(96, 127.0)], kickless_bars: Some((40, 56)), seed: 11, ..Spec::default() };
    let e = eval(&spec, false);
    assert!((bpm_of(&e.grid) - 127.0).abs() < 0.005);
    assert!(e.max_err_ms < 1.0, "max err {}", e.max_err_ms);
    assert!(e.downbeat_ok);
}

#[test]
fn tempo_change_gives_piecewise_grid() {
    let spec = Spec { sections: vec![(48, 120.0), (48, 126.0)], seed: 21, ..Spec::default() };
    let e = eval(&spec, false);
    assert!(matches!(e.grid.map, BeatMap::Piecewise(_)), "{:?}", e.grid.map);
    assert!(e.max_err_ms < 2.0, "max err {}", e.max_err_ms);
    assert!(e.count_ok);
}

#[test]
fn live_drift_gives_live_map_that_follows() {
    let spec = Spec { sections: vec![(64, 118.0)], jitter_ms: 4.0, drift_per_bar: 0.004, seed: 31, ..Spec::default() };
    let e = eval(&spec, true);
    assert!(matches!(e.grid.map, BeatMap::Live(_)), "{:?}", e.grid.map);
    println!("live: max err vs pulse {:.2} ms, p95 {:.2} ms", e.max_err_ms, e.p95_ms);
    assert!(e.max_err_ms < 8.0, "max err vs pulse {}", e.max_err_ms);
    assert!(e.count_ok);
}

#[test]
fn key_is_detected() {
    for (pc, minor, seed) in [(9, true, 41), (0, false, 42), (7, true, 43), (2, false, 44)] {
        let key = Key::new(pc, minor);
        let r = synth::render(&Spec { sections: vec![(32, 125.0)], key, seed, ..Spec::default() });
        let out = analyze(&r.audio, &AnalysisConfig::default());
        assert_eq!(out.analysis.key, Some(key), "{}", key.musical());
    }
}
