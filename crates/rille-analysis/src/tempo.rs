//! Global tempo estimation: autocorrelation + comb filter, octave choice by
//! on-beat / off-beat contrast within the user's BPM range.

use realfft::RealFftPlanner;

#[derive(Clone, Debug)]
pub struct TempoEstimate {
    pub bpm: f64,
    /// Contrast of the chosen tempo divided by that of half or double tempo
    /// (> 1; close to 1 means ambiguous; 10 when neither is in range).
    pub octave_margin: f64,
}

pub fn estimate(odf: &[f32], fps: f64, range: (f64, f64)) -> Option<TempoEstimate> {
    if odf.len() < (fps * 8.0) as usize {
        return None;
    }
    let acf = autocorrelation(odf, (fps * 60.0 / 40.0 * 4.0 + 4.0) as usize);
    let at = |lag: f64| -> f64 {
        let i = lag.floor() as usize;
        if i + 1 >= acf.len() {
            return 0.0;
        }
        let f = lag - i as f64;
        acf[i] * (1.0 - f) + acf[i + 1] * f
    };
    let comb = |bpm: f64| -> f64 {
        let tau = 60.0 * fps / bpm;
        (1..=4).map(|k| at(k as f64 * tau)).sum()
    };

    // Coarse-to-fine search for the strongest periodicity.
    let mut best = (0.0, f64::MIN);
    let mut bpm = 50.0;
    while bpm <= 240.0 {
        let s = comb(bpm);
        if s > best.1 {
            best = (bpm, s);
        }
        bpm += 0.05;
    }
    let mut lo = best.0 - 0.05;
    let hi = best.0 + 0.05;
    while lo <= hi {
        let s = comb(lo);
        if s > best.1 {
            best = (lo, s);
        }
        lo += 0.002;
    }

    // The comb can peak at a sub- or super-harmonic (e.g. 58 for 174). Try
    // simple ratios inside the range and keep the tempo whose beats land on
    // the strongest onsets.
    let mut options: Vec<f64> = Vec::new();
    for r in [1.0, 2.0, 0.5, 3.0, 1.0 / 3.0, 4.0, 0.25, 1.5, 2.0 / 3.0, 0.75, 4.0 / 3.0] {
        let b = best.0 * r;
        if b >= range.0 * 0.995 && b <= range.1 * 1.005 && options.iter().all(|o| (o / b - 1.0).abs() > 0.01) {
            options.push(b);
        }
    }
    if options.is_empty() {
        options.push(fold_into(best.0, range));
    }
    let sal: Vec<f64> = options.iter().map(|&b| salience(odf, fps, b)).collect();
    let chosen = (0..options.len()).max_by(|&a, &b| sal[a].total_cmp(&sal[b])).unwrap_or(0);
    // Only half or double tempo is a real ambiguity for the DJ (the ×2/÷2
    // buttons); a 3:2 alternative is never close in dance music.
    let octave = |b: f64| [2.0, 0.5].iter().any(|r| (b / options[chosen] / r - 1.0).abs() < 0.01);
    let runner_up = sal
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != chosen && octave(options[*i]))
        .map(|(_, v)| *v)
        .fold(0.0, f64::max);
    let margin = if runner_up > 0.0 { sal[chosen] / runner_up } else { 10.0 };
    Some(TempoEstimate { bpm: options[chosen], octave_margin: margin })
}

/// Constant-tempo lattice that best stacks the onsets of the whole track:
/// folds the onset curve at candidate tempos (±0.4 % around `bpm0`, 0.002
/// BPM steps) and returns `(bpm, phase_secs, peak_to_mean)` of the sharpest
/// fold. Only the exact tempo keeps every kick in the same phase bin across
/// several minutes, so this is precise (drift < one bin over the track) and
/// immune to local tracking mistakes.
pub fn lattice(odf: &[f32], fps: f64, bpm0: f64) -> (f64, f64, f64) {
    const BINS: usize = 64;
    let mut best = (bpm0, 0.0, 0.0);
    let steps = (bpm0 * 0.008 / 0.002).ceil() as i64;
    for s in -steps / 2..=steps / 2 {
        let bpm = bpm0 + s as f64 * 0.002;
        let period = 60.0 * fps / bpm;
        let mut hist = [0.0f64; BINS];
        let mut ph = 0.0f64;
        let scale = BINS as f64 / period;
        for v in odf {
            hist[((ph * scale) as usize).min(BINS - 1)] += f64::from(*v);
            ph += 1.0;
            if ph >= period {
                ph -= period;
            }
        }
        let mean = hist.iter().sum::<f64>() / BINS as f64;
        if mean <= 0.0 {
            continue;
        }
        let (bin, peak) = (0..BINS)
            .map(|i| (i, hist[(i + BINS - 1) % BINS] + 2.0 * hist[i] + hist[(i + 1) % BINS]))
            .fold((0, f64::MIN), |m, (i, v)| if v > m.1 { (i, v) } else { m });
        let sharpness = peak / (4.0 * mean);
        if sharpness > best.2 {
            // Parabolic peak position within the bin.
            let (y0, y1, y2) = (hist[(bin + BINS - 1) % BINS], hist[bin], hist[(bin + 1) % BINS]);
            let d = y0 - 2.0 * y1 + y2;
            let frac = if d.abs() > 1e-12 { (0.5 * (y0 - y2) / d).clamp(-0.5, 0.5) } else { 0.0 };
            let phase_frames = (bin as f64 + 0.5 + frac) / scale;
            best = (bpm, phase_frames / fps, sharpness);
        }
    }
    best
}

fn fold_into(mut bpm: f64, range: (f64, f64)) -> f64 {
    while bpm < range.0 && bpm * 2.0 <= range.1 * 1.01 {
        bpm *= 2.0;
    }
    while bpm > range.1 && bpm / 2.0 >= range.0 * 0.99 {
        bpm /= 2.0;
    }
    bpm
}

/// Mean over 20 s windows of the onset strength at the best beat phase,
/// relative to the overall mean: how well beats at this tempo sit on onsets.
/// Windows keep small tempo errors from smearing the phase.
pub fn salience(odf: &[f32], fps: f64, bpm: f64) -> f64 {
    let tau = 60.0 * fps / bpm;
    let win = (20.0 * fps) as usize;
    let overall = odf.iter().map(|v| f64::from(*v)).sum::<f64>() / odf.len().max(1) as f64;
    let mut total = 0.0;
    let mut count = 0;
    let mut start = 0;
    while start + win <= odf.len() {
        let seg = &odf[start..start + win];
        let bins = tau.ceil() as usize;
        let mut hist = vec![0.0f64; bins];
        let mut cnt = vec![0u32; bins];
        for (i, v) in seg.iter().enumerate() {
            let ph = (i as f64 % tau).floor() as usize % bins;
            hist[ph] += f64::from(*v);
            cnt[ph] += 1;
        }
        let h: Vec<f64> = hist.iter().zip(&cnt).map(|(s, c)| s / f64::from((*c).max(1))).collect();
        // Smooth over ±1 bin to tolerate phase drift.
        let peak =
            (0..bins).map(|i| (h[(i + bins - 1) % bins] + h[i] + h[(i + 1) % bins]) / 3.0).fold(f64::MIN, f64::max);
        total += peak;
        count += 1;
        start += win / 2;
    }
    if count == 0 || overall <= 0.0 { 0.0 } else { total / f64::from(count) / overall }
}

fn autocorrelation(x: &[f32], max_lag: usize) -> Vec<f64> {
    let n = (x.len() * 2).next_power_of_two();
    let mut planner = RealFftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    let mean = x.iter().map(|v| f64::from(*v)).sum::<f64>() / x.len() as f64;
    let mut buf = fwd.make_input_vec();
    for (b, v) in buf.iter_mut().zip(x) {
        *b = f64::from(*v) - mean;
    }
    let mut spec = fwd.make_output_vec();
    fwd.process(&mut buf, &mut spec).expect("fft sizes match");
    for c in spec.iter_mut() {
        *c = realfft::num_complex::Complex::new(c.norm_sqr(), 0.0);
    }
    inv.process(&mut spec, &mut buf).expect("fft sizes match");
    let zero = buf[0].max(1e-12);
    buf[..max_lag.min(n)].iter().map(|v| v / zero).collect()
}
