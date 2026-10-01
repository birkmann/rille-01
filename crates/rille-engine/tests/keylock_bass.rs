//! The keylock stretcher must not move low frequencies in time: a kick's
//! bass body has to land where it does without keylock.
use rille_core::{BeatGrid, BeatMap, Control, ControlEvent, ControlTarget, ControlValue, GridSource};
use rille_engine::{Command, HOTCUES, LoadedTrack, TrackAudio, create};
use std::sync::Arc;

const SR: u32 = 48_000;

fn kick_track(bpm: f64, secs: f64) -> Arc<TrackAudio> {
    let n = (secs * f64::from(SR)) as usize;
    let mut frames = vec![[0.0f32; 2]; n];
    let beat = 60.0 / bpm;
    let mut t = 0.5;
    while t < secs - 1.0 {
        let s = (t * f64::from(SR)) as usize;
        for k in 0..(0.25 * f64::from(SR)) as usize {
            let x = k as f64 / f64::from(SR);
            let v = (2.0 * std::f64::consts::PI * 55.0 * x).sin() * (-x * 14.0).exp();
            frames[s + k] = [v as f32 * 0.8; 2];
        }
        t += beat;
    }
    Arc::new(TrackAudio { sample_rate: SR, frames })
}

/// Output time (s) where the low-band envelope of each kick first exceeds half its peak.
fn kick_times(out: &[[f32; 2]]) -> Vec<f64> {
    let env: Vec<f32> = out.iter().map(|f| f[0].abs()).collect();
    let mut times = Vec::new();
    let mut i = 0;
    while i < env.len() {
        if env[i] > 0.25 {
            times.push(i as f64 / f64::from(SR));
            i += (0.3 * f64::from(SR)) as usize;
        } else {
            i += 1;
        }
    }
    times
}

#[test]
fn keylock_keeps_bass_on_time() {
    let mut at = Vec::new();
    for keylock in [false, true] {
        let (h, mut e) = create(SR, 1024);
        let audio = kick_track(120.0, 60.0);
        let grid = Arc::new(BeatGrid::new(BeatMap::constant(0.5, 120.0).unwrap(), GridSource::Manual));
        let track = LoadedTrack {
            id: 1,
            audio,
            grid: Some(grid),
            main_cue_secs: 0.0,
            hotcues: [None; HOTCUES],
            auto_gain_db: 0.0,
        };
        let _ = h.send(Command::Load { deck: 0, track });
        let press = |c| {
            for d in [true, false] {
                let _ = h.send(Command::Control(ControlEvent {
                    target: ControlTarget::deck(0, c),
                    value: ControlValue::Press(d),
                }));
            }
        };
        if !keylock {
            press(Control::Keylock);
        }
        press(Control::Play);
        let mut out = Vec::new();
        for _ in 0..(20.0 * f64::from(SR) / 256.0) as usize {
            let (m, _) = e.render(256);
            out.extend_from_slice(m);
        }
        let times = kick_times(&out);
        // Expected: kick n at 0.5 + n*0.5 s plus the limiter's 1 ms.
        let errs: Vec<f64> = times
            .iter()
            .skip(2)
            .map(|t| {
                let p = ((t - 0.001 - 0.5) / 0.5).round();
                (t - 0.001 - (0.5 + p * 0.5)) * 1000.0
            })
            .collect();
        let mean = errs.iter().sum::<f64>() / errs.len() as f64;
        eprintln!("keylock {keylock}: bass body at {mean:+.2} ms");
        at.push(mean);
    }
    assert!((at[1] - at[0]).abs() < 1.0, "keylock moves the bass by {:.2} ms", at[1] - at[0]);
}
