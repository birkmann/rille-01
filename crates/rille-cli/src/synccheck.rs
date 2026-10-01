//! `rille-cli synccheck <master> <follower> [secs]`: plays two real tracks
//! through the engine offline, the follower synced, and measures how far
//! apart their beats actually sound. The master deck goes to the main
//! output, the follower to the headphone bus, so each can be measured on
//! its own: kick onsets (low band) of both are cross-correlated in 4 s
//! windows. Also prints the engine's own phase reading and how often sync
//! jumped the follower into phase.

use std::path::Path;
use std::sync::Arc;

use rille_analysis::{AnalysisConfig, analyze_file};
use rille_core::{BeatClock, Control, ControlEvent, ControlTarget, ControlValue};
use rille_engine::{Command, EngineHandle, HOTCUES, LoadedTrack, TrackAudio, create};

const SR: u32 = 48_000;
const BLOCK: usize = 256;
/// The main output passes the limiter (1 ms look-ahead), the cue bus does not.
const LIMITER_FRAMES: usize = 48;

fn ctl(h: &EngineHandle, deck: u8, c: Control, v: ControlValue) {
    let _ = h.send(Command::Control(ControlEvent { target: ControlTarget::deck(deck, c), value: v }));
}

fn global(h: &EngineHandle, c: Control, v: ControlValue) {
    let _ = h.send(Command::Control(ControlEvent { target: ControlTarget::global(c), value: v }));
}

fn press(h: &EngineHandle, deck: u8, c: Control) {
    ctl(h, deck, c, ControlValue::Press(true));
    ctl(h, deck, c, ControlValue::Press(false));
}

/// Kick onset strength at 1 kHz: rise of the low-band (≈ 150 Hz) energy.
fn onsets(x: &[f32]) -> Vec<f32> {
    let a = (-2.0 * std::f32::consts::PI * 150.0 / SR as f32).exp();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    let hop = (SR / 1000) as usize;
    let mut env = Vec::with_capacity(x.len() / hop);
    let mut acc = 0.0f32;
    for (i, v) in x.iter().enumerate() {
        s1 = (1.0 - a) * v + a * s1;
        s2 = (1.0 - a) * s1 + a * s2;
        acc += s2 * s2;
        if (i + 1) % hop == 0 {
            env.push((acc / hop as f32).sqrt());
            acc = 0.0;
        }
    }
    let mut out = vec![0.0; env.len()];
    for i in 1..env.len() {
        out[i] = (env[i] - env[i - 1]).max(0.0);
    }
    out
}

/// Where in the beat the onsets of `x` (1 kHz) pile up: fold at the beat
/// length and return the start of the steepest rise of the folded profile
/// (ms into the beat) and how clear the fold is (peak over mean).
fn beat_phase(x: &[f32], beat_ms: f64) -> (f64, f32) {
    let bins = beat_ms.round() as usize;
    let mut fold = vec![0.0f32; bins];
    for (i, v) in x.iter().enumerate() {
        fold[((i as f64) % beat_ms) as usize % bins] += v;
    }
    // Smooth over 3 ms (circular).
    let sm: Vec<f32> = (0..bins).map(|i| (0..5).map(|k| fold[(i + bins + k - 2) % bins]).sum::<f32>() / 5.0).collect();
    let (peak, max) = sm.iter().enumerate().fold((0, f32::MIN), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
    let mean = sm.iter().sum::<f32>() / bins as f32;
    (peak as f64, if max > 0.0 { max / mean.max(1e-12) } else { 0.0 })
}

/// Lag (ms) of `b` against `a` with the best correlation within ±`max` ms,
/// and how clear it is (0..1).
#[allow(dead_code)]
fn best_lag(a: &[f32], b: &[f32], max: i64) -> (i64, f32) {
    let corr = |lag: i64| -> f32 {
        let mut s = 0.0;
        for (i, &x) in a.iter().enumerate() {
            let j = i as i64 + lag;
            if j >= 0 && (j as usize) < b.len() {
                s += x * b[j as usize];
            }
        }
        s
    };
    let vals: Vec<(i64, f32)> = (-max..=max).map(|l| (l, corr(l))).collect();
    let (lag, best) = vals.iter().copied().fold((0, f32::MIN), |m, v| if v.1 > m.1 { v } else { m });
    let mean = vals.iter().map(|v| v.1).sum::<f32>() / vals.len() as f32;
    (lag, if best > 0.0 { ((best - mean) / best).clamp(0.0, 1.0) } else { 0.0 })
}

pub fn run(master: &Path, follower: &Path, secs: f64, start: (f64, f64), cfg: &AnalysisConfig) {
    let load = |p: &Path| {
        let (audio, out) = analyze_file(p, cfg, None).unwrap_or_else(|e| {
            eprintln!("{}: {e}", p.display());
            std::process::exit(1);
        });
        let grid = out.analysis.grid.map(Arc::new);
        // Where the measurement's reference (the folded low-band onset peak)
        // sits relative to the grid in the file itself, 40–100 s in.
        let offset = grid.as_ref().map_or(0.0, |g| {
            let mono: Vec<f32> = audio.frames.iter().map(|f| 0.5 * (f[0] + f[1])).collect();
            let ratio = f64::from(SR) / f64::from(audio.sample_rate);
            let resampled: Vec<f32> = (0..(mono.len() as f64 * ratio) as usize)
                .map(|i| mono[((i as f64 / ratio) as usize).min(mono.len() - 1)])
                .collect();
            let on = onsets(&resampled[(40 * SR) as usize..(100 * SR) as usize]);
            let beat_ms = 60_000.0 / g.bpm_at(60.0);
            let (peak, _) = beat_phase(&on, beat_ms);
            // Grid line phase at 40 s, in ms into the beat.
            let b = g.beat_at(40.0);
            let line = (b.ceil() - b) * beat_ms;
            (peak - line + beat_ms * 1.5).rem_euclid(beat_ms) - beat_ms * 0.5
        });
        println!(
            "{}: {:.3} bpm, reference {:+.1} ms from the grid",
            p.display(),
            grid.as_ref().map_or(0.0, |g| g.bpm_at(60.0)),
            offset
        );
        (Arc::new(TrackAudio { sample_rate: audio.sample_rate, frames: audio.frames }), grid)
    };
    // RILLE_SYNC_CLICK_MASTER=<bpm>: the master is a click track with an exact
    // grid instead of `master`, so the follower is measured against clicks.
    let (a_audio, a_grid) = match std::env::var("RILLE_SYNC_CLICK_MASTER").ok().and_then(|v| v.parse::<f64>().ok()) {
        Some(bpm) => {
            let secs = 400.0;
            let mut frames = vec![[0.0f32; 2]; (secs * f64::from(SR)) as usize];
            let mut t = 0.0;
            while t < secs - 0.01 {
                let i = (t * f64::from(SR)).round() as usize;
                for f in frames.iter_mut().skip(i).take(48) {
                    *f = [0.9, 0.9];
                }
                t += 60.0 / bpm;
            }
            let grid = rille_core::BeatGrid::new(
                rille_core::BeatMap::constant(0.0, bpm).expect("valid"),
                rille_core::GridSource::Manual,
            );
            println!("master: click track at {bpm} bpm");
            (Arc::new(TrackAudio { sample_rate: SR, frames }), Some(Arc::new(grid)))
        }
        None => load(master),
    };
    let (b_audio, b_grid) = load(follower);
    let (h, mut e) = create(SR, 1024);
    let sources = [a_audio.clone(), b_audio.clone()];
    for (deck, audio, grid, cue) in [(0u8, a_audio, a_grid.clone(), start.0), (1, b_audio, b_grid.clone(), start.1)] {
        let track = LoadedTrack {
            id: u64::from(deck) + 1,
            audio,
            grid,
            main_cue_secs: cue,
            hotcues: [None; HOTCUES],
            auto_gain_db: 0.0,
        };
        let _ = h.send(Command::Load { deck, track });
    }
    // Deck B only on the headphone bus, deck A only on the main output.
    ctl(&h, 1, Control::Volume, ControlValue::Absolute(0.0));
    press(&h, 1, Control::Pfl);
    global(&h, Control::CueMix, ControlValue::Absolute(0.0));
    global(&h, Control::Limiter, ControlValue::Press(true));

    let (mut main, mut cue): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    let render = |e: &mut rille_engine::Engine, s: f64, main: &mut Vec<f32>, cue: &mut Vec<f32>| {
        for _ in 0..(s * f64::from(SR) / BLOCK as f64) as usize {
            let (m, c) = e.render(BLOCK);
            main.extend(m.iter().map(|f| 0.5 * (f[0] + f[1])));
            cue.extend(c.iter().map(|f| 0.5 * (f[0] + f[1])));
        }
    };
    // RILLE_SYNC_VARISPEED=1: follower without keylock (no time-stretching).
    let varispeed = std::env::var_os("RILLE_SYNC_VARISPEED").is_some();
    if varispeed {
        press(&h, 0, Control::Keylock);
        press(&h, 1, Control::Keylock);
    }
    press(&h, 0, Control::Play);
    render(&mut e, 1.0, &mut main, &mut cue);
    press(&h, 1, Control::Sync);
    press(&h, 1, Control::Play);
    println!("\n time   engine phase B−A   heard B−A (fold clarity A/B)   realigns   tempo B");
    let mut engine_phase = Vec::new();
    let t0 = main.len();
    let mut t = 0.0;
    while t < secs {
        render(&mut e, 1.0, &mut main, &mut cue);
        t += 1.0;
        let s = h.snapshot();
        let (da, db) = (&s.decks[0], &s.decks[1]);
        let beat_ms = 60_000.0 / da.bpm.max(1.0);
        let d = (db.beat - da.beat).rem_euclid(1.0);
        let d = if d > 0.5 { d - 1.0 } else { d };
        engine_phase.push(d * beat_ms);
        if (t as i64) % 4 == 0 && t >= 8.0 {
            let w = 8 * SR as usize;
            let end = main.len();
            // The main output is 1 ms later than the cue bus.
            let a = onsets(&main[end - w + LIMITER_FRAMES..end]);
            let b = onsets(&cue[end - w..end - LIMITER_FRAMES]);
            let (pa, ca) = beat_phase(&a, beat_ms);
            let (pb, cb) = beat_phase(&b, beat_ms);
            let lag = (pb - pa + beat_ms * 1.5).rem_euclid(beat_ms) - beat_ms * 0.5;
            println!(
                "{t:5.0} s   {:+8.2} ms        {:+6.1} ms ({ca:.1}/{cb:.1})        {:5}     {:.2} bpm ({:+.1} %)",
                d * beat_ms,
                lag,
                db.realigns,
                db.bpm,
                (db.rate - 1.0) * 100.0
            );
        }
    }
    let _ = t0;
    if varispeed {
        // Where each deck really is: rebuild its output from the source at
        // the positions the engine reports and find the lag that matches.
        let mut pos: Vec<[f64; 2]> = Vec::new();
        let first = main.len();
        let before = h.snapshot();
        pos.push([before.decks[0].position_secs, before.decks[1].position_secs]);
        for _ in 0..(2 * SR as usize / BLOCK) {
            let (m, c) = e.render(BLOCK);
            main.extend(m.iter().map(|f| 0.5 * (f[0] + f[1])));
            cue.extend(c.iter().map(|f| 0.5 * (f[0] + f[1])));
            let s = h.snapshot();
            pos.push([s.decks[0].position_secs, s.decks[1].position_secs]);
        }
        for d in 0..2 {
            let src = &sources[d];
            let sr = f64::from(src.sample_rate);
            let out: &[f32] = if d == 0 { &main[first + LIMITER_FRAMES..] } else { &cue[first..] };
            let n = (pos.len() - 1) * BLOCK - LIMITER_FRAMES;
            let predicted: Vec<f32> = (0..n)
                .map(|i| {
                    let (b, f) = (i / BLOCK, (i % BLOCK) as f64 / BLOCK as f64);
                    let t = pos[b][d] + (pos[b + 1][d] - pos[b][d]) * f;
                    let x = t * sr;
                    let k = x.floor().max(0.0) as usize;
                    let g = |j: usize| src.frames.get(j).map_or(0.0, |v| 0.5 * (v[0] + v[1]));
                    g(k) + (g(k + 1) - g(k)) * (x - x.floor()) as f32
                })
                .collect();
            let corr = |lag: i64| -> f64 {
                (2000..n - 2000)
                    .step_by(2)
                    .map(|i| f64::from(predicted[i]) * f64::from(out[(i as i64 + lag) as usize]))
                    .sum()
            };
            let (lag, _) =
                (-1500..=1500).map(|l| (l, corr(l))).fold((0, f64::MIN), |m, v| if v.1 > m.1 { v } else { m });
            println!(
                "deck {}: heard {:+.2} ms from where the engine says it is",
                ["A", "B"][d],
                lag as f64 * 1000.0 / f64::from(SR)
            );
        }
    }
    // RILLE_SYNC_WAV=<file>: write both decks mixed (as heard), for listening.
    if let Some(path) = std::env::var_os("RILLE_SYNC_WAV") {
        let mut bytes = Vec::with_capacity(44 + main.len() * 4);
        let data_len = (main.len().min(cue.len()) * 4) as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&SR.to_le_bytes());
        bytes.extend_from_slice(&(SR * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for (i, c) in cue.iter().enumerate().take(main.len()) {
            // Main is 1 ms behind the cue bus; line them up.
            let m = main.get(i + LIMITER_FRAMES).copied().unwrap_or(0.0);
            bytes.extend_from_slice(&(0.5 * (m + c)).to_le_bytes());
        }
        if let Err(e) = std::fs::write(&path, bytes) {
            eprintln!("{}: {e}", std::path::Path::new(&path).display());
        }
    }
    let n = engine_phase.len().max(1) as f64;
    let worst = engine_phase.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    println!("\nengine phase: mean {:+.2} ms, worst {:.2} ms", engine_phase.iter().sum::<f64>() / n, worst);
}
