//! Builds the three-band waveform summary in one streaming pass.

use rille_core::waveform::{WAVEFORM_BINS_PER_SEC, WaveformSummary};

struct Lp {
    a: f32,
    z: f32,
}

impl Lp {
    fn new(sr: f64, f: f64) -> Self {
        Self { a: (1.0 - (-2.0 * std::f64::consts::PI * f / sr).exp()) as f32, z: 0.0 }
    }

    fn tick(&mut self, x: f32) -> f32 {
        self.z += self.a * (x - self.z);
        self.z
    }
}

pub fn build(frames: &[[f32; 2]], sr: f64) -> WaveformSummary {
    let per_bin = sr / WAVEFORM_BINS_PER_SEC;
    let n_bins = (frames.len() as f64 / per_bin).ceil() as usize;
    let mut bins = Vec::with_capacity(n_bins);
    // Two cascaded one-poles per split: gentle, but plenty for display.
    let (mut lo1, mut lo2) = (Lp::new(sr, 250.0), Lp::new(sr, 250.0));
    let (mut hi1, mut hi2) = (Lp::new(sr, 3000.0), Lp::new(sr, 3000.0));
    let mut peak = [0.0f32; 4];
    let mut next_edge = per_bin;
    let q = |v: f32| (255.0 * v.min(1.0).sqrt()).round() as u8;
    for (i, [l, r]) in frames.iter().enumerate() {
        let x = 0.5 * (l + r);
        let low = lo2.tick(lo1.tick(x));
        let below_hi = hi2.tick(hi1.tick(x));
        let high = x - below_hi;
        let mid = below_hi - low;
        peak[0] = peak[0].max(low.abs());
        peak[1] = peak[1].max(mid.abs());
        peak[2] = peak[2].max(high.abs());
        peak[3] = peak[3].max(x.abs());
        if (i + 1) as f64 >= next_edge {
            bins.push(peak.map(q));
            peak = [0.0; 4];
            next_edge += per_bin;
        }
    }
    if bins.len() < n_bins {
        bins.push(peak.map(q));
    }
    WaveformSummary { bins_per_sec: WAVEFORM_BINS_PER_SEC, bins }
}
