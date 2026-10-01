//! `rille-cli`: developer tool.
//!
//! ```text
//! rille-cli analyze <files...>                  grid report per file
//! rille-cli play <file> [seconds] [--buffer N]   play on the default audio device
//! rille-cli gridplot <out-dir> <files...>        beat/bar stack PNG per file
//! rille-cli click <out-dir> <files...>           track + metronome click WAV per file
//! rille-cli eval <files or dirs...> [--mixxx <mixxxdb.sqlite>] [--bpm-range lo:hi]
//! rille-cli audit <files or dirs...>             grid vs audio, independent of the analyzer
//! ```
//! `eval` compares BPM and key with the files' tags and, when given, with a
//! Mixxx database (BPM and first-beat phase).

mod audit;
mod eval;
mod mixxx;
mod plot;
mod synccheck;

use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;
use rille_analysis::{AnalysisConfig, analyze_file};
use rille_core::{BeatClock, BeatMap};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        usage();
    };
    let mut files = Vec::new();
    let mut mixxx: Option<PathBuf> = None;
    let mut buffer: Option<u32> = None;
    let mut seconds = 10.0;
    let mut cfg = AnalysisConfig::default();
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--mixxx" => mixxx = rest.next().map(PathBuf::from),
            "--buffer" => buffer = rest.next().and_then(|b| b.parse().ok()),
            s if s.parse::<f64>().is_ok() => seconds = s.parse().unwrap_or(10.0),
            "--bpm-range" => {
                let r = rest
                    .next()
                    .and_then(|s| s.split_once(':'))
                    .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)));
                cfg.bpm_range = r.unwrap_or_else(|| usage());
            }
            _ => collect_audio(Path::new(a), &mut files),
        }
    }
    files.sort();
    match cmd.as_str() {
        "analyze" => files.iter().for_each(|f| analyze_one(f, &cfg)),
        "eval" => eval::run(&files, &cfg, mixxx.as_deref()),
        "audit" => audit::run(&files, &cfg),
        "chroma-dump" => eval::chroma_dump(&files),
        "play" => play(files.first().unwrap_or_else(|| usage()), seconds, buffer, &cfg),
        "synccheck" => {
            let (Some(a), Some(b)) = (files.first(), files.get(1)) else { usage() };
            // Keep the order given on the command line: master first.
            let pick = |i: usize| args[1..].iter().filter(|x| Path::new(x).is_file()).nth(i).map(PathBuf::from);
            let (a, b) = (pick(0).unwrap_or_else(|| a.clone()), pick(1).unwrap_or_else(|| b.clone()));
            synccheck::run(&a, &b, seconds, (51.0, 59.0), &cfg);
        }
        "gridplot" | "click" => {
            let out_dir = PathBuf::from(args.get(1).unwrap_or_else(|| usage()));
            std::fs::create_dir_all(&out_dir).ok();
            let mut results: Vec<String> = files
                .par_iter()
                .filter(|f| !f.starts_with(&out_dir))
                .map(|f| {
                    let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    if cmd == "gridplot" {
                        plot::gridplot(f, &out_dir.join(format!("{stem}.png")), &cfg)
                            .unwrap_or_else(|e| format!("{}: {e}", f.display()))
                    } else {
                        match plot::click(f, &out_dir.join(format!("{stem}.click.wav")), &cfg) {
                            Ok(()) => format!("{stem}.click.wav"),
                            Err(e) => format!("{}: {e}", f.display()),
                        }
                    }
                })
                .collect();
            results.sort();
            results.iter().for_each(|r| println!("{r}"));
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("usage: rille-cli analyze <files...> | eval <files/dirs...> [--mixxx db] [--bpm-range lo:hi]");
    std::process::exit(2)
}

fn collect_audio(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                collect_audio(&e.path(), out);
            }
        }
    } else if rille_library::is_supported(p) {
        out.push(p.to_path_buf());
    }
}

fn analyze_one(path: &Path, cfg: &AnalysisConfig) {
    let t = Instant::now();
    match analyze_file(path, cfg, None) {
        Ok((_, out)) => {
            let a = &out.analysis;
            let r = &out.report;
            println!("{}", path.display());
            if let Some(g) = &a.grid {
                let first = g.secs_at(0.0);
                let kind = match &g.map {
                    BeatMap::Constant(_) => "constant".to_string(),
                    BeatMap::Piecewise(m) => format!("piecewise ({} segments)", m.segments().len()),
                    BeatMap::Live(m) => format!("live ({} beats)", m.beats_secs().len()),
                };
                println!(
                    "  grid     {kind}, {:.3} bpm (fit {:.4}), first downbeat {:.4} s, confidence {:.2}, flags {:?}",
                    r.bpm, r.bpm_fit, first, g.confidence, g.flags
                );
            } else {
                println!("  grid     none");
            }
            println!(
                "  fit      rms {:.3} ms, drift {:.3} ms, hits {:.1}% (p50 {:.2} ms, p95 {:.2} ms), {} strong beats, band {}",
                r.rms_ms,
                r.drift_ms,
                r.hit_rate * 100.0,
                r.hit_p50_ms,
                r.hit_p95_ms,
                r.strong_beats,
                r.band
            );
            println!(
                "  other    key {} ({}) conf {:.2}, {:.1} LUFS, peak {:.1} dB, octave margin {:.2}, downbeat margin {:.2}, {:.2} s",
                a.key.map_or("-".into(), |k| k.musical()),
                a.key.map_or("-".into(), |k| k.camelot()),
                a.key_confidence,
                a.lufs.unwrap_or(f32::NAN),
                a.peak_db.unwrap_or(f32::NAN),
                r.octave_margin,
                r.downbeat_margin,
                t.elapsed().as_secs_f64()
            );
        }
        Err(e) => println!("{}: error: {e}", path.display()),
    }
}

/// Plays a file through the engine on the default output device.
fn play(path: &Path, seconds: f64, buffer: Option<u32>, cfg: &AnalysisConfig) {
    use rille_core::{Control, ControlEvent, ControlTarget, ControlValue};
    use rille_engine::{Command, HOTCUES, LoadedTrack, TrackAudio};
    let (audio, out) = match analyze_file(path, cfg, None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let config = rille_engine::backend::AudioConfig { device: None, buffer_frames: buffer };
    let (engine, output) = match rille_engine::backend::start(&config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("audio: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "device {} · {} Hz · {} channels · buffer {:?}",
        output.device_name, output.sample_rate, output.channels, output.buffer_frames
    );
    let track = LoadedTrack {
        id: 1,
        audio: std::sync::Arc::new(TrackAudio { sample_rate: audio.sample_rate, frames: audio.frames }),
        grid: out.analysis.grid.map(std::sync::Arc::new),
        main_cue_secs: 0.0,
        hotcues: [None; HOTCUES],
        auto_gain_db: out.analysis.lufs.map_or(0.0, |l| (-10.0 - l).clamp(-12.0, 12.0)),
    };
    let _ = engine.send(Command::Load { deck: 0, track });
    for down in [true, false] {
        let _ = engine.send(Command::Control(ControlEvent {
            target: ControlTarget::deck(0, Control::Play),
            value: ControlValue::Press(down),
        }));
    }
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        engine.poll(|_| {});
        let s = engine.snapshot();
        println!(
            "{:5.1} s  pos {:7.3} s  bpm {:7.2}  cpu {:4.1} %  peak {:.2}",
            start.elapsed().as_secs_f64(),
            s.decks[0].position_secs,
            s.decks[0].bpm,
            s.cpu_load * 100.0,
            s.master_meter[0].max(s.master_meter[1])
        );
    }
    drop(output);
}
