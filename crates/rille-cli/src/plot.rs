//! Visual grid checks: `gridplot` renders beat and bar stacks as a PNG,
//! `click` writes the track with a metronome click on every grid beat.

use std::path::Path;

use image::{Rgb, RgbImage};
use rille_analysis::quality::kick_alignment;
use rille_analysis::refine::{Band, TransientFinder};
use rille_analysis::{AnalysisConfig, analyze_file};
use rille_core::{BeatClock, BeatGrid};

const HALF_MS: f64 = 60.0;
const COLS: usize = 241; // −60…+60 ms in 0.5 ms steps
const BAR_COL: usize = 48; // pixels per beat in the bar stack

fn norm_row(v: &[f32]) -> Vec<f32> {
    let max = v.iter().copied().fold(1e-12f32, f32::max);
    v.iter().map(|x| (x / max).sqrt()).collect()
}

/// Beat stack (low band | broadband, one row per beat, ±60 ms, kick starts
/// marked white, grid line red, downbeats tagged amber on the left) and bar
/// stack (one row per bar, four beats wide, kick-band energy).
pub fn gridplot(path: &Path, out: &Path, cfg: &AnalysisConfig) -> Result<String, String> {
    let (audio, res) = analyze_file(path, cfg, None).map_err(|e| e.to_string())?;
    let grid: BeatGrid = res.analysis.grid.ok_or("no grid")?;
    let sr = f64::from(audio.sample_rate);
    let mono = audio.mono();
    let dur = audio.duration_secs();
    let low = TransientFinder::new(&mono, sr, Band::Low);
    let broad = TransientFinder::new(&mono, sr, Band::Broad);
    let (b0, b1) = (grid.beat_at(0.1).ceil() as i64, grid.beat_at(dur - 0.1).floor() as i64);
    let beats: Vec<i64> = (b0..=b1).collect();

    let bar_first = (b0 - grid.downbeat_beat_index).div_euclid(4) * 4 + grid.downbeat_beat_index;
    let bars: Vec<i64> = (0..).map(|k| bar_first + 4 * k).take_while(|b| *b <= b1).collect();
    let bar_row_h = 3usize;
    let x_low = 10usize;
    let x_broad = x_low + COLS + 6;
    let x_bar = x_broad + COLS + 14;
    let width = x_bar + 4 * BAR_COL + 4;
    let height = beats.len().max(bars.len() * bar_row_h).max(10);
    let mut img = RgbImage::from_pixel(width as u32, height as u32, Rgb([8, 9, 11]));
    let put = |img: &mut RgbImage, x: usize, y: usize, c: [u8; 3]| {
        if x < width && y < height {
            img.put_pixel(x as u32, y as u32, Rgb(c));
        }
    };

    for (row, &b) in beats.iter().enumerate() {
        let g = grid.secs_at(b as f64);
        let half = HALF_MS / 1000.0;
        let lo = norm_row(&low.envelope(g, half, COLS));
        let br = norm_row(&broad.envelope(g, half, COLS));
        for c in 0..COLS {
            let v = lo[c];
            put(&mut img, x_low + c, row, [(v * 60.0) as u8, (v * 140.0) as u8, (v * 255.0) as u8]);
            let v = br[c];
            put(&mut img, x_broad + c, row, [(v * 255.0) as u8, (v * 150.0) as u8, (v * 40.0) as u8]);
        }
        let mid = COLS / 2;
        put(&mut img, x_low + mid, row, [220, 40, 40]);
        put(&mut img, x_broad + mid, row, [220, 40, 40]);
        if let Some(t) = low.attack(g, 0.03) {
            let c = ((t.secs - g) * 1000.0 / 0.5).round() as isize + mid as isize;
            if (0..COLS as isize).contains(&c) {
                put(&mut img, x_low + c as usize, row, [255, 255, 255]);
            }
        }
        if grid.is_downbeat(b) {
            for x in 2..8 {
                put(&mut img, x, row, [245, 165, 36]);
            }
        }
    }

    // Bar stack: log kick-band energy across each bar.
    let env_per_bar: Vec<Vec<f32>> = bars
        .iter()
        .map(|&bar| {
            (0..4 * BAR_COL)
                .map(|c| {
                    let beat = bar as f64 + c as f64 / BAR_COL as f64;
                    let t = grid.secs_at(beat);
                    low.envelope(t, 0.004, 3).iter().sum::<f32>() / 3.0
                })
                .collect()
        })
        .collect();
    let peak = env_per_bar.iter().flatten().copied().fold(1e-12f32, f32::max);
    for (r, row) in env_per_bar.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            let db = 10.0 * (v / peak).max(1e-6).log10();
            let s = ((db + 40.0) / 40.0).clamp(0.0, 1.0);
            let col = [(s * 90.0) as u8, (s * 170.0) as u8, (s * 255.0) as u8];
            for y in 0..bar_row_h - 1 {
                put(&mut img, x_bar + c, r * bar_row_h + y, col);
            }
        }
        for beat in 0..=4 {
            let x = x_bar + beat * BAR_COL;
            let col = if beat == 0 || beat == 4 { [245, 165, 36] } else { [70, 70, 70] };
            for y in 0..bar_row_h {
                put(&mut img, x.min(width - 1), r * bar_row_h + y, col);
            }
        }
    }
    img.save(out).map_err(|e| e.to_string())?;
    let k = kick_alignment(&low, &grid, dur);
    Ok(format!(
        "{}: {:.3} bpm {:?}, kicks {}/{} beats, kick-band rise {:+.2} ms (within 2 ms {:.1} %, p95 {:.2} ms), beat-to-grid p95 {:.2} ms, flags {:?}",
        path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        res.report.bpm,
        res.report.kind,
        k.kicks,
        k.beats,
        k.offset_ms,
        k.hit_2ms * 100.0,
        k.p95_ms,
        res.report.hit_p95_ms,
        grid.flags
    ))
}

/// Writes the track (16-bit WAV) with a click on every grid beat, accented
/// on downbeats, so the grid can be checked by ear.
pub fn click(path: &Path, out: &Path, cfg: &AnalysisConfig) -> Result<(), String> {
    use std::io::Write;
    let (audio, res) = analyze_file(path, cfg, None).map_err(|e| e.to_string())?;
    let grid = res.analysis.grid.ok_or("no grid")?;
    let sr = f64::from(audio.sample_rate);
    let mut frames = audio.frames;
    let dur = frames.len() as f64 / sr;
    let (b0, b1) = (grid.beat_at(0.0).ceil() as i64, grid.beat_at(dur).floor() as i64);
    for b in b0..=b1 {
        let start = (grid.secs_at(b as f64) * sr).round() as usize;
        let (f, amp) = if grid.is_downbeat(b) { (2000.0, 0.5) } else { (1200.0, 0.3) };
        for k in 0..(0.012 * sr) as usize {
            let t = k as f64 / sr;
            let v = ((2.0 * std::f64::consts::PI * f * t).sin() * (-t * 400.0).exp() * amp) as f32;
            if let Some(fr) = frames.get_mut(start + k) {
                fr[0] = fr[0] * 0.7 + v;
                fr[1] = fr[1] * 0.7 + v;
            }
        }
    }
    let mut f = std::io::BufWriter::new(std::fs::File::create(out).map_err(|e| e.to_string())?);
    let n = frames.len() as u32;
    let data = n * 4;
    let rate = audio.sample_rate;
    let header: Vec<u8> = [
        b"RIFF".to_vec(),
        (36 + data).to_le_bytes().to_vec(),
        b"WAVEfmt ".to_vec(),
        16u32.to_le_bytes().to_vec(),
        1u16.to_le_bytes().to_vec(),
        2u16.to_le_bytes().to_vec(),
        rate.to_le_bytes().to_vec(),
        (rate * 4).to_le_bytes().to_vec(),
        4u16.to_le_bytes().to_vec(),
        16u16.to_le_bytes().to_vec(),
        b"data".to_vec(),
        data.to_le_bytes().to_vec(),
    ]
    .concat();
    f.write_all(&header).map_err(|e| e.to_string())?;
    for [l, r] in frames {
        for v in [l, r] {
            f.write_all(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
