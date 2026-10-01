//! Onset detection functions (spectral flux) at ~172 frames per second.
//!
//! Frame `t` is centred on sample `t * hop`, so frame times are exact.

use realfft::RealFftPlanner;

pub struct Onsets {
    pub fps: f64,
    pub hop: usize,
    /// Broadband log-spectral flux, normalized.
    pub all: Vec<f32>,
    /// Kick band (30–150 Hz) flux, normalized.
    pub low: Vec<f32>,
    /// `all + low`: what the beat tracker follows.
    pub combined: Vec<f32>,
}

impl Onsets {
    pub fn frame_secs(&self, frame: f64) -> f64 {
        frame / self.fps
    }
}

pub fn compute(mono: &[f32], sr: u32) -> Onsets {
    let sr_f = f64::from(sr);
    let hop = (sr_f / 172.265625).round().max(1.0) as usize;
    let win = (4 * hop).next_power_of_two();
    let n_bins = win / 2 + 1;
    let bin_hz = sr_f / win as f64;
    let bin = |hz: f64| ((hz / bin_hz).round() as usize).min(n_bins - 1);
    let (low_a, low_b) = (bin(30.0).max(1), bin(150.0).max(2));
    let (all_a, all_b) = (bin(30.0).max(1), bin(16_000.0f64.min(sr_f * 0.45)));

    let window: Vec<f32> =
        (0..win).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / win as f64).cos()) as f32).collect();
    let norm = 2.0 / window.iter().sum::<f32>();

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(win);
    let mut input = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();

    let n_frames = mono.len() / hop + 1;
    let mut prev = vec![0.0f32; n_bins];
    let mut cur = vec![0.0f32; n_bins];
    let mut all = Vec::with_capacity(n_frames);
    let mut low = Vec::with_capacity(n_frames);

    for t in 0..n_frames {
        let centre = (t * hop) as isize;
        for (i, v) in input.iter_mut().enumerate() {
            let idx = centre - (win / 2) as isize + i as isize;
            *v = if idx >= 0 && (idx as usize) < mono.len() { mono[idx as usize] * window[i] } else { 0.0 };
        }
        fft.process_with_scratch(&mut input, &mut spec, &mut scratch).expect("fft sizes match");
        for (c, s) in cur.iter_mut().zip(&spec) {
            *c = (1.0 + 100.0 * s.norm() * norm).ln();
        }
        let flux = |a: usize, b: usize| -> f32 { (a..=b).map(|k| (cur[k] - prev[k]).max(0.0)).sum() };
        all.push(if t == 0 { 0.0 } else { flux(all_a, all_b) });
        low.push(if t == 0 { 0.0 } else { flux(low_a, low_b) });
        std::mem::swap(&mut prev, &mut cur);
    }

    let all = normalize(&all);
    let low = normalize(&low);
    let combined = all.iter().zip(&low).map(|(a, l)| a + l).collect();
    Onsets { fps: sr_f / hop as f64, hop, all, low, combined }
}

/// Subtracts a local mean (±8 frames), rectifies and scales to unit std.
fn normalize(x: &[f32]) -> Vec<f32> {
    const R: usize = 8;
    let n = x.len();
    let mut prefix = vec![0.0f64; n + 1];
    for (i, v) in x.iter().enumerate() {
        prefix[i + 1] = prefix[i] + f64::from(*v);
    }
    let mut y: Vec<f32> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(R), (i + R + 1).min(n));
            let mean = (prefix[b] - prefix[a]) / (b - a) as f64;
            (f64::from(x[i]) - mean).max(0.0) as f32
        })
        .collect();
    let mean = y.iter().map(|v| f64::from(*v)).sum::<f64>() / n.max(1) as f64;
    let var = y.iter().map(|v| (f64::from(*v) - mean).powi(2)).sum::<f64>() / n.max(1) as f64;
    let sd = var.sqrt().max(1e-9) as f32;
    y.iter_mut().for_each(|v| *v /= sd);
    y
}
