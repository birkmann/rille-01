//! Independent grid audit: checks each grid against the audio without using
//! any of the analyzer's own beat observations.
//!
//! The analyzer's quality numbers measure attacks *near* its lines, so a grid
//! that is a sixteenth off, or a beat detector locked onto the wrong sound,
//! still scores well there. The audit folds the kick-band energy over the
//! whole beat (±half a beat around every line) and asks where the big rise
//! really is, in the whole track and in each eighth of it (tempo drift).

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rille_analysis::filter::{Biquad, filtfilt};
use rille_analysis::{AnalysisConfig, analyze_file};
use rille_core::{BeatClock, BeatGrid};

/// Profile resolution.
const STEP: f64 = 0.001;

pub struct Audit {
    /// Kick onset (punch × click, see `rille_analysis::kick`) relative to the line (ms).
    pub kick_ms: f64,
    /// Low-band power gained within 6 ms at the kick, as a share of the
    /// beat's peak, and the broadband step there (dB).
    pub punch: f64,
    pub click_db: f64,
    /// Steepest broadband rise relative to the line (ms) and the rise at the
    /// line (±12 ms) relative to it.
    pub broad_ms: f64,
    pub broad_at_line: f64,
    /// Kick position per eighth of the track (ms), where the kick is clear.
    pub sections_ms: Vec<f64>,
}

impl Audit {
    pub fn clear_kick(&self) -> bool {
        self.click_db >= 4.0 && self.punch >= 0.05 || self.click_db >= 8.0 && self.punch >= 0.02
    }

    /// Largest distance between the kick positions of two sections (ms).
    pub fn drift_ms(&self) -> f64 {
        let (lo, hi) = self.sections_ms.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        if self.sections_ms.len() < 2 { 0.0 } else { hi - lo }
    }

    pub fn verdict(&self) -> &'static str {
        if self.clear_kick() {
            if self.kick_ms.abs() > 15.0 {
                "OFF-KICK"
            } else if self.drift_ms() > 12.0 {
                "DRIFT"
            } else {
                "ok"
            }
        } else if self.broad_ms.abs() > 15.0 && self.broad_at_line < 0.8 {
            "OFF-ONSET"
        } else {
            "ok-nokick"
        }
    }
}

/// Band power, zero-phase filtered, averaged to one value per `STEP`.
fn envelope(mono: &[f32], sr: f64, hp: f64, lp: f64) -> Vec<f32> {
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let mut x = mono.to_vec();
    let mut f = vec![Biquad::highpass(sr, hp, q)];
    if lp > 0.0 {
        f.push(Biquad::lowpass(sr, lp, q));
    }
    filtfilt(&f, &mut x);
    // Exact bounds per step (44.1 samples per ms: whole chunks would drift).
    let n = (x.len() as f64 / (STEP * sr)) as usize;
    (0..n)
        .map(|k| {
            let (a, b) = ((k as f64 * STEP * sr) as usize, (((k + 1) as f64 * STEP * sr) as usize).min(x.len()));
            x[a..b].iter().map(|v| v * v).sum::<f32>() / (b - a).max(1) as f32
        })
        .collect()
}

/// Average over `lines` of each beat's envelope from −half to +half a beat,
/// each beat normalised to its maximum, louder beats counting more.
fn profile(env: &[f32], lines: &[f64], half: f64) -> Vec<f64> {
    let n = (2.0 * half / STEP) as usize;
    let at = |t: f64| -> f64 {
        let x = t / STEP;
        let i = x.floor();
        if i < 0.0 || i as usize + 1 >= env.len() {
            return 0.0;
        }
        let f = x - i;
        f64::from(env[i as usize]) * (1.0 - f) + f64::from(env[i as usize + 1]) * f
    };
    let rows: Vec<Vec<f64>> =
        lines.iter().map(|&g| (0..n).map(|k| at(g - half + k as f64 * STEP)).collect::<Vec<f64>>()).collect();
    let mut peaks: Vec<f64> = rows.iter().map(|r| r.iter().copied().fold(0.0, f64::max)).collect();
    peaks.sort_by(f64::total_cmp);
    let med = peaks.get(peaks.len() / 2).copied().unwrap_or(0.0).max(1e-20);
    let mut acc = vec![0.0; n];
    for r in &rows {
        let max = r.iter().copied().fold(1e-20, f64::max);
        let w = (max / med).min(1.0);
        acc.iter_mut().zip(r).for_each(|(a, v)| *a += w * v / max);
    }
    acc
}

/// (position of the steepest rise in ms relative to the line, rise at the
/// line / steepest rise). The rise is measured over 8 ms; the profile wraps
/// around (it covers exactly one beat).
fn steepest(p: &[f64], half: f64) -> (f64, f64) {
    let n = p.len();
    let d = (0.004 / STEP) as usize;
    let rise = |i: usize| p[(i + d) % n] - p[(i + n - d) % n];
    let best = (0..n).max_by(|&a, &b| rise(a).total_cmp(&rise(b))).unwrap_or(0);
    let centre = n / 2;
    let w = (0.012 / STEP) as usize;
    let near = (centre - w..=centre + w).map(rise).fold(f64::MIN, f64::max);
    let top = rise(best).max(1e-12);
    ((best as f64 * STEP - half) * 1000.0, (near / top).max(0.0))
}

/// Mean log energy (dB) around every line, −half..+half, one value per `STEP`.
fn fold_db(env: &[f32], lines: &[f64], half: f64) -> Vec<f64> {
    let n = (2.0 * half / STEP) as usize;
    let mean = env.iter().map(|v| f64::from(*v)).sum::<f64>() / env.len().max(1) as f64;
    let floor = mean * 1e-4 + 1e-20;
    let mut acc = vec![0.0; n];
    for &g in lines {
        for (k, a) in acc.iter_mut().enumerate() {
            let i = ((g - half) / STEP).round() as isize + k as isize;
            let v = env.get(i.max(0) as usize).copied().unwrap_or(0.0);
            *a += 10.0 * (f64::from(v) + floor).log10();
        }
    }
    acc.iter_mut().for_each(|a| *a /= lines.len().max(1) as f64);
    acc
}

/// Kick-band power at `STEP` resolution as x² + (x'/ω)² (no ripple from
/// the low tone itself).
fn low_envelope(mono: &[f32], sr: f64) -> Vec<f32> {
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let mut x = mono.to_vec();
    filtfilt(&[Biquad::highpass(sr, 30.0, q), Biquad::lowpass(sr, 200.0, q)], &mut x);
    let w = (2.0 * std::f64::consts::PI * 60.0 / sr) as f32;
    let n = (x.len() as f64 / (STEP * sr)) as usize;
    (0..n)
        .map(|k| {
            let (a, b) = ((k as f64 * STEP * sr) as usize, (((k + 1) as f64 * STEP * sr) as usize).min(x.len() - 1));
            (a.max(1)..b.max(2)).map(|i| x[i] * x[i] + ((x[i + 1] - x[i - 1]) * 0.5 / w).powi(2)).sum::<f32>()
                / (b - a).max(1) as f32
        })
        .collect()
}

/// Mean linear power around every line, −half..+half, one value per `STEP`.
fn fold_lin(env: &[f32], lines: &[f64], half: f64) -> Vec<f64> {
    let n = (2.0 * half / STEP) as usize;
    let mut acc = vec![0.0; n];
    for &g in lines {
        for (k, a) in acc.iter_mut().enumerate() {
            let i = ((g - half) / STEP).round() as isize + k as isize;
            *a += f64::from(env.get(i.max(0) as usize).copied().unwrap_or(0.0));
        }
    }
    acc
}

/// (kick onset ms relative to the line, punch, click dB): the sharpest
/// broadband step that has low-band power arriving with it (the scoring of `rille_analysis::kick`, measured here around
/// the final grid lines from independently filtered signals).
fn kick_at(lin: &[f64], broad: &[f64], half: f64) -> (f64, f64, f64) {
    let n = lin.len() as isize;
    let peak = lin.iter().copied().fold(1e-20, f64::max);
    let at = |p: &[f64], i: isize| p[i.rem_euclid(n) as usize];
    let mean = |p: &[f64], a: isize, b: isize| (a..=b).map(|i| at(p, i)).sum::<f64>() / (b - a + 1) as f64;
    let step = |j: isize| mean(broad, j, j + 3) - mean(broad, j - 8, j - 4);
    let (mut best, mut best_s, mut bp, mut cd) = (0isize, f64::MIN, 0.0, 0.0);
    for i in 0..n {
        let punch = (mean(lin, i + 1, i + 6) - mean(lin, i - 10, i - 3)) / peak;
        let (cj, click) =
            (i - 3..=i + 6).map(|j| (j, step(j))).fold((i, f64::MIN), |m, x| if x.1 > m.1 { x } else { m });
        let s = click.max(0.0) * punch.max(0.0).sqrt();
        if s > best_s {
            (best, best_s, bp, cd) = (cj, s, punch, click);
        }
    }
    let ms = (best as f64 * STEP - half) * 1000.0;
    let period_ms = 2.0 * half * 1000.0;
    (ms - (ms / period_ms).round() * period_ms, bp, cd)
}

pub fn audit_grid(mono: &[f32], sr: f64, grid: &BeatGrid, duration: f64) -> Audit {
    let low = low_envelope(mono, sr);
    let broad = envelope(mono, sr, 200.0, 0.0);
    let (b0, b1) = (grid.beat_at(0.5).ceil() as i64, grid.beat_at(duration - 0.5).floor() as i64);
    let lines: Vec<f64> = (b0..=b1).map(|b| grid.secs_at(b as f64)).collect();
    let half = 30.0 / grid.bpm_at(duration / 2.0);
    let (kick_ms, punch, click_db) = kick_at(&fold_lin(&low, &lines, half), &fold_db(&broad, &lines, half), half);
    let (broad_ms, broad_at_line) = steepest(&profile(&broad, &lines, half), half);
    let sections_ms = lines
        .chunks(lines.len().div_ceil(8).max(16))
        .filter(|c| c.len() >= 16)
        .filter_map(|c| {
            let (ms, b, k) = kick_at(&fold_lin(&low, c, half), &fold_db(&broad, c, half), half);
            (b >= 0.05 && k >= 4.0 && (ms - kick_ms).abs() < 40.0).then_some(ms)
        })
        .collect();
    Audit { kick_ms, punch, click_db, broad_ms, broad_at_line, sections_ms }
}

pub fn run(files: &[PathBuf], cfg: &AnalysisConfig) {
    let mut rows: Vec<(PathBuf, f64, Audit)> = files
        .par_iter()
        .filter_map(|path| {
            let (audio, out) = analyze_file(path, cfg, None).map_err(|e| eprintln!("{}: {e}", path.display())).ok()?;
            let grid = out.analysis.grid?;
            let a = audit_grid(&audio.mono(), f64::from(audio.sample_rate), &grid, audio.duration_secs());
            print_row(path, out.report.bpm, &a);
            Some((path.clone(), out.report.bpm, a))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let n = rows.len();
    println!("\n== audit of {n} tracks");
    for v in ["ok", "ok-nokick", "OFF-KICK", "OFF-ONSET", "DRIFT"] {
        println!("  {v:9} {}", rows.iter().filter(|r| r.2.verdict() == v).count());
    }
    let mut k: Vec<f64> = rows.iter().filter(|r| r.2.verdict() == "ok").map(|r| r.2.kick_ms).collect();
    k.sort_by(f64::total_cmp);
    if !k.is_empty() {
        println!(
            "  kick onset on ok tracks: median {:+.1} ms, p5..p95 {:+.1}..{:+.1} ms",
            k[k.len() / 2],
            k[k.len() * 5 / 100],
            k[(k.len() * 95 / 100).min(k.len() - 1)]
        );
    }
    println!("  failures:");
    for (p, bpm, a) in rows.iter().filter(|r| !r.2.verdict().starts_with("ok")) {
        print!("  ");
        print_row(p, *bpm, a);
    }
}

fn print_row(path: &Path, bpm: f64, a: &Audit) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    println!(
        "{:9} {:8.3} kick {:+6.1} ms (punch {:4.2} click {:4.1} dB) broad {:+6.1} ms ({:4.2}) drift {:5.1} ms/{} | {name}",
        a.verdict(),
        bpm,
        a.kick_ms,
        a.punch,
        a.click_db,
        a.broad_ms,
        a.broad_at_line,
        a.drift_ms(),
        a.sections_ms.len(),
    );
}
