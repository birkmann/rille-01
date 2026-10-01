//! Batch evaluation against tag BPM/key and a Mixxx database.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use rayon::prelude::*;
use rille_analysis::quality::{KickAlignment, kick_alignment};
use rille_analysis::refine::{Band, TransientFinder};
use rille_analysis::{AnalysisConfig, analyze_file};
use rille_core::{BeatClock, BeatMap, GridFlags, Key};

use crate::mixxx;

struct Row {
    path: PathBuf,
    ours_bpm: f64,
    kind: &'static str,
    confidence: f32,
    flags: GridFlags,
    rms_ms: f64,
    hit_rate: f64,
    hit_p95_ms: f64,
    tag_bpm: Option<f64>,
    mixxx_bpm: Option<f64>,
    /// Our grid vs Mixxx first beat, wrapped to ±half a beat (ms).
    mixxx_phase_ms: Option<f64>,
    key: Option<Key>,
    tag_key: Option<Key>,
    secs: f64,
    phase_source: &'static str,
    kick: KickAlignment,
    /// Where the grid lines fall within the beat 60 s in (ms), to compare runs.
    grid_phase_ms: f64,
    downbeat: (&'static str, f64, usize),
    /// Seconds of the first downbeat, to compare runs.
    first_downbeat: f64,
}

pub fn run(files: &[PathBuf], cfg: &AnalysisConfig, mixxx_db: Option<&Path>) {
    let mixxx = mixxx_db.map(|p| mixxx::load(p).expect("readable Mixxx database")).unwrap_or_default();
    let cache = std::env::temp_dir().join(format!("rille-eval-{}", std::process::id()));
    std::fs::create_dir_all(&cache).ok();
    let lib = Mutex::new(rille_library::Library::open_in_memory(&cache).expect("in-memory library"));

    let started = Instant::now();
    let rows: Vec<Row> = files
        .par_iter()
        .filter_map(|path| {
            let tags = {
                let mut l = lib.lock().expect("library lock");
                l.import_file(path).ok().and_then(|id| l.track(id).ok().flatten())
            };
            let t = Instant::now();
            let (audio, out) = analyze_file(path, cfg, None).map_err(|e| eprintln!("{}: {e}", path.display())).ok()?;
            let secs = t.elapsed().as_secs_f64();
            let grid = out.analysis.grid.as_ref()?;
            let m = mixxx.get(path);
            let mixxx_phase_ms = m.and_then(|m| m.first_beat_secs).map(|fb| {
                let b = grid.beat_at(fb);
                let beat_len = 60.0 / grid.bpm_at(fb);
                (b - b.round()) * beat_len * 1000.0
            });
            let low = TransientFinder::new(&audio.mono(), f64::from(audio.sample_rate), Band::Low);
            let kick = kick_alignment(&low, grid, audio.duration_secs());
            let grid_phase_ms = {
                let b = grid.beat_at(60.0);
                (b - b.floor()) * 60.0 / grid.bpm_at(60.0) * 1000.0
            };
            let row = Row {
                path: path.clone(),
                ours_bpm: out.report.bpm,
                kind: match grid.map {
                    BeatMap::Constant(_) => "constant",
                    BeatMap::Piecewise(_) => "piecewise",
                    BeatMap::Live(_) => "live",
                },
                confidence: grid.confidence,
                flags: grid.flags,
                rms_ms: out.report.rms_ms,
                hit_rate: out.report.hit_rate,
                hit_p95_ms: out.report.hit_p95_ms,
                tag_bpm: tags.as_ref().and_then(|t| t.bpm),
                mixxx_bpm: m.map(|m| m.bpm),
                mixxx_phase_ms,
                key: out.analysis.key,
                tag_key: tags.as_ref().and_then(|t| t.key),
                secs,
                phase_source: out.report.phase_source,
                kick,
                grid_phase_ms,
                downbeat: (out.report.downbeat_source, out.report.downbeat_agreement, out.report.downbeat_events),
                first_downbeat: grid.secs_at(0.0),
            };
            print_row(&row);
            Some(row)
        })
        .collect();
    std::fs::remove_dir_all(&cache).ok();
    summary(&rows, started.elapsed().as_secs_f64());
}

fn print_row(r: &Row) {
    let name = r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let f = |v: Option<f64>| v.map_or("     -".to_string(), |v| format!("{v:7.2}"));
    println!(
        "{:8.3} {:9} conf {:.2} rms {:5.2} hit {:5.1}% p95 {:5.2} | kick {:+6.2} ms {:5.1}% ph {:6.1} db {:8.3}s {:15} {:4.2} ev {:2} | tag {} mixxx {} phase {} | key {:4} tag {:4} | {:.1}s {}{}",
        r.ours_bpm,
        r.kind,
        r.confidence,
        r.rms_ms,
        r.hit_rate * 100.0,
        r.hit_p95_ms,
        r.kick.offset_ms,
        r.kick.hit_2ms * 100.0,
        r.grid_phase_ms,
        r.first_downbeat,
        r.downbeat.0,
        r.downbeat.1,
        r.downbeat.2,
        f(r.tag_bpm),
        f(r.mixxx_bpm),
        r.mixxx_phase_ms.map_or("    -".into(), |p| format!("{p:6.1}")),
        r.key.map_or("-".into(), |k| k.camelot()),
        r.tag_key.map_or("-".into(), |k| k.camelot()),
        r.secs,
        name,
        if r.flags.is_empty() { String::new() } else { format!("  {:?}", r.flags) },
    );
}

/// Relation between two tempos: exact, same up to rounding, octave, other.
fn bpm_relation(ours: f64, other: f64) -> &'static str {
    if (ours - other).abs() < 0.02 {
        "exact"
    } else if (ours - other).abs() < 0.51 {
        "close"
    } else if [2.0, 0.5, 1.5, 2.0 / 3.0].iter().any(|r| (ours - other * r).abs() < 0.6) {
        "octave"
    } else {
        "wrong"
    }
}

fn summary(rows: &[Row], wall: f64) {
    let n = rows.len();
    println!(
        "\n== {n} tracks in {wall:.1} s ({:.2} s per track CPU)",
        rows.iter().map(|r| r.secs).sum::<f64>() / n.max(1) as f64
    );
    let count = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count();
    for kind in ["constant", "piecewise", "live"] {
        println!("  {kind:10} {}", count(&|r| r.kind == kind));
    }
    println!("  low confidence (<0.6): {}", count(&|r| r.confidence < 0.6));
    for (name, flag) in GridFlags::all().iter_names() {
        println!("  flag {name:20} {}", count(&|r| r.flags.contains(flag)));
    }
    for (label, get) in [("tag BPM", 0usize), ("Mixxx BPM", 1)] {
        let pairs: Vec<&str> = rows
            .iter()
            .filter_map(|r| if get == 0 { r.tag_bpm } else { r.mixxx_bpm }.map(|o| bpm_relation(r.ours_bpm, o)))
            .collect();
        if pairs.is_empty() {
            continue;
        }
        let c = |k: &str| pairs.iter().filter(|p| **p == k).count();
        println!(
            "  vs {label:9} ({:3}): exact {}  close {}  octave {}  wrong {}",
            pairs.len(),
            c("exact"),
            c("close"),
            c("octave"),
            c("wrong")
        );
    }
    for src in ["kick (both bands)", "kick band", "broadband attack", ""] {
        println!(
            "  phase from {:18} {}",
            if src.is_empty() { "(no fix)" } else { src },
            count(&|r| r.phase_source == src)
        );
    }
    for src in ["section changes", "novelty"] {
        let rs: Vec<&Row> = rows.iter().filter(|r| r.downbeat.0 == src).collect();
        let agree = rs.iter().filter(|r| r.downbeat.2 >= 2 && r.downbeat.1 >= 0.999).count();
        println!("  downbeat from {src:16} {:4} (all section changes agree on {agree})", rs.len());
    }
    let kicked: Vec<&Row> = rows.iter().filter(|r| r.kick.kicks >= 32).collect();
    if !kicked.is_empty() {
        let mut offs: Vec<f64> = kicked.iter().map(|r| r.kick.offset_ms).collect();
        offs.sort_by(f64::total_cmp);
        let mut hits: Vec<f64> = kicked.iter().map(|r| r.kick.hit_2ms).collect();
        hits.sort_by(f64::total_cmp);
        let k = kicked.len();
        println!(
            "  kick-band rise vs grid ({k} tracks with kicks; late on kicks whose sweep starts above 200 Hz): offset median {:+.2} ms, p5..p95 {:+.2}..{:+.2} ms, |offset| <= 1 ms {}; within 2 ms median {:.1} %, >= 95 % on {}",
            offs[k / 2],
            offs[k * 5 / 100],
            offs[(k * 95 / 100).min(k - 1)],
            offs.iter().filter(|o| o.abs() <= 1.0).count(),
            hits[k / 2] * 100.0,
            hits.iter().filter(|h| **h >= 0.95).count()
        );
    }
    let mut exact_phases: Vec<f64> = rows
        .iter()
        .filter(|r| r.mixxx_bpm.is_some_and(|m| bpm_relation(r.ours_bpm, m) == "exact"))
        .filter_map(|r| r.mixxx_phase_ms)
        .collect();
    exact_phases.sort_by(f64::total_cmp);
    if exact_phases.len() >= 4 {
        let q = |f: f64| exact_phases[((exact_phases.len() - 1) as f64 * f).round() as usize];
        println!(
            "  Mixxx first beat vs ours (exact-BPM tracks, {}): median {:+.1} ms, IQR {:+.1}..{:+.1} ms, p10..p90 {:+.1}..{:+.1} ms",
            exact_phases.len(),
            q(0.5),
            q(0.25),
            q(0.75),
            q(0.1),
            q(0.9)
        );
    }
    let phases: Vec<f64> = rows
        .iter()
        .filter(|r| r.mixxx_bpm.is_some_and(|m| bpm_relation(r.ours_bpm, m) != "wrong"))
        .filter_map(|r| r.mixxx_phase_ms)
        .collect();
    if !phases.is_empty() {
        let within = |ms: f64| phases.iter().filter(|p| p.abs() < ms).count();
        println!(
            "  phase vs Mixxx ({}): <5 ms {}  <10 ms {}  <20 ms {}",
            phases.len(),
            within(5.0),
            within(10.0),
            within(20.0)
        );
    }
    let mut hits: Vec<f64> = rows.iter().map(|r| r.hit_p95_ms).collect();
    hits.sort_by(f64::total_cmp);
    if !hits.is_empty() {
        println!(
            "  transient-to-grid p95 per track: median {:.2} ms, worst {:.2} ms; tracks with p95 < 2 ms: {}/{n}",
            hits[n / 2],
            hits[n - 1],
            hits.iter().filter(|h| **h < 2.0).count()
        );
    }
    let keyed: Vec<(Key, Key)> = rows.iter().filter_map(|r| Some((r.key?, r.tag_key?))).collect();
    if !keyed.is_empty() {
        let exact = keyed.iter().filter(|(a, b)| a == b).count();
        let related = keyed
            .iter()
            .filter(|(a, b)| {
                a != b
                    && (a.camelot_number() == b.camelot_number() || {
                        let d = (i16::from(a.camelot_number()) - i16::from(b.camelot_number())).rem_euclid(12);
                        a.is_minor() == b.is_minor() && (d == 1 || d == 11)
                    })
            })
            .count();
        println!("  key vs tag ({}): exact {exact}  relative/fifth {related}", keyed.len());
    }
}

/// Prints `tag_key_index path` followed by several global chroma variants
/// (12 values each) for tracks with a key tag, for tuning key detection.
pub fn chroma_dump(files: &[PathBuf]) {
    use rille_analysis::key::{ChromaOpts, chroma_with, total};
    let cache = std::env::temp_dir().join(format!("rille-chroma-{}", std::process::id()));
    std::fs::create_dir_all(&cache).ok();
    let lib = Mutex::new(rille_library::Library::open_in_memory(&cache).expect("in-memory library"));
    let d = ChromaOpts::default();
    let variants = [
        ChromaOpts { log: true, peaks_only: true, ..d },
        ChromaOpts { whiten: true, ..d },
        ChromaOpts { whiten: true, sustain: true, ..d },
        ChromaOpts { whiten: true, sustain: true, peaks_only: true, ..d },
        ChromaOpts { whiten: true, sustain: true, lo_hz: 110.0, hi_hz: 1800.0, ..d },
        ChromaOpts { whiten: true, sustain: true, win: 8192, lo_hz: 50.0, ..d },
    ];
    files.par_iter().for_each(|path| {
        let tag = {
            let mut l = lib.lock().expect("library lock");
            l.import_file(path).ok().and_then(|id| l.track(id).ok().flatten()).and_then(|t| t.key)
        };
        let Some(tag) = tag else { return };
        let Ok(audio) = rille_decode::decode_file(path, None, &mut |_| {}) else { return };
        let (x, sr) = rille_analysis::decimate_for_key(&audio.mono(), f64::from(audio.sample_rate));
        let mut line = format!("{}", u8::from(tag));
        for v in &variants {
            for c in total(&chroma_with(&x, sr, *v)) {
                line.push_str(&format!(" {c:.4}"));
            }
        }
        println!("{line}");
    });
    std::fs::remove_dir_all(&cache).ok();
}
