//! Chromagram and key detection (profile correlation).

use realfft::RealFftPlanner;
use rille_core::Key;

pub struct Chroma {
    /// Seconds per chroma frame; frame `i` is centred at `i * hop_secs`.
    pub hop_secs: f64,
    pub frames: Vec<[f32; 12]>,
}

impl Chroma {
    /// Mean chroma over `[a, b)` seconds.
    pub fn mean(&self, a: f64, b: f64) -> [f32; 12] {
        let i0 = (a / self.hop_secs).round().max(0.0) as usize;
        let i1 = ((b / self.hop_secs).round() as usize).max(i0 + 1).min(self.frames.len());
        let mut out = [0.0f32; 12];
        for f in self.frames.get(i0..i1).unwrap_or(&[]) {
            for k in 0..12 {
                out[k] += f[k];
            }
        }
        out
    }
}

/// Chromagram settings (defaults are what key detection uses).
#[derive(Clone, Copy, Debug)]
pub struct ChromaOpts {
    pub win: usize,
    pub lo_hz: f64,
    pub hi_hz: f64,
    /// Log-compress magnitudes (`ln(1 + 100·m)`), reducing the dominance of
    /// the loudest partials (usually the bass).
    pub log: bool,
    /// Only spectral peaks contribute (suppresses broadband drum noise).
    pub peaks_only: bool,
    /// Keep only what rises above the local spectral floor (log domain),
    /// removing noise and drum spectra that cover many bins.
    pub whiten: bool,
    /// Keep only energy present in three consecutive frames: notes sustain,
    /// drum hits don't.
    pub sustain: bool,
}

impl Default for ChromaOpts {
    fn default() -> Self {
        Self { win: 4096, lo_hz: 65.0, hi_hz: 2000.0, log: false, peaks_only: false, whiten: true, sustain: true }
    }
}

/// `x` is mono audio at `sr` (a decimated rate around 11–12 kHz is ideal).
pub fn chroma(x: &[f32], sr: f64) -> Chroma {
    chroma_with(x, sr, ChromaOpts::default())
}

pub fn chroma_with(x: &[f32], sr: f64, opts: ChromaOpts) -> Chroma {
    let win = opts.win;
    let hop = win / 2;
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(win);
    let mut input = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let window: Vec<f32> =
        (0..win).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / win as f64).cos()) as f32).collect();
    let norm = 2.0 / window.iter().sum::<f32>();
    // Pitch class per bin (A = 9) within the frequency range.
    let bin_pc: Vec<Option<usize>> = (0..=win / 2)
        .map(|k| {
            let f = k as f64 * sr / win as f64;
            (opts.lo_hz..opts.hi_hz)
                .contains(&f)
                .then(|| ((12.0 * (f / 440.0).log2()).round() as i64 + 9).rem_euclid(12) as usize)
        })
        .collect();
    let n_bins = win / 2 + 1;
    let mut mags = vec![0.0f32; n_bins];
    let mut prefix = vec![0.0f32; n_bins + 1];
    // Per-frame bin values, kept for the sustain minimum over 3 frames.
    let mut spectra: Vec<Vec<f32>> = Vec::new();
    let mut centre = 0usize;
    while centre < x.len() {
        for (i, v) in input.iter_mut().enumerate() {
            let idx = centre as isize - (win / 2) as isize + i as isize;
            *v = if idx >= 0 && (idx as usize) < x.len() { x[idx as usize] * window[i] } else { 0.0 };
        }
        fft.process(&mut input, &mut spec).expect("fft sizes match");
        for (m, s) in mags.iter_mut().zip(&spec) {
            let v = s.norm() * norm;
            *m = if opts.log || opts.whiten { (1.0 + 100.0 * v).ln() } else { v };
        }
        if opts.whiten {
            for k in 0..n_bins {
                prefix[k + 1] = prefix[k] + mags[k];
            }
            let floor: Vec<f32> = (0..n_bins)
                .map(|k| {
                    let w = (k / 12).max(4);
                    let (a, b) = (k.saturating_sub(w), (k + w + 1).min(n_bins));
                    (prefix[b] - prefix[a]) / (b - a) as f32
                })
                .collect();
            for (m, f) in mags.iter_mut().zip(&floor) {
                *m = (*m - f).max(0.0);
            }
        }
        let mut frame = vec![0.0f32; n_bins];
        for k in 1..n_bins - 1 {
            if bin_pc[k].is_some() && (!opts.peaks_only || (mags[k] > mags[k - 1] && mags[k] >= mags[k + 1])) {
                frame[k] = mags[k];
            }
        }
        spectra.push(frame);
        centre += hop;
    }
    let frames = (0..spectra.len())
        .map(|t| {
            let mut c = [0.0f32; 12];
            for (k, pc) in bin_pc.iter().enumerate() {
                let Some(pc) = pc else { continue };
                let v = if opts.sustain && t > 0 && t + 1 < spectra.len() {
                    spectra[t - 1][k].min(spectra[t][k]).min(spectra[t + 1][k])
                } else {
                    spectra[t][k]
                };
                c[*pc] += v;
            }
            c
        })
        .collect();
    Chroma { hop_secs: hop as f64 / sr, frames }
}

/// Sum of all frames.
pub fn total(ch: &Chroma) -> [f64; 12] {
    let mut total = [0.0f64; 12];
    for f in &ch.frames {
        for k in 0..12 {
            total[k] += f64::from(f[k]);
        }
    }
    total
}

/// Key profiles learned from 390 electronic tracks with Beatport key tags,
/// on whitened, sustained chroma (see `rille-cli chroma-dump`). Held-out
/// accuracy against the tags: 55 % exact, a further 16 % relative or fifth
/// (still harmonically compatible). Classic probe-tone profiles reach 36–48 %
/// on the same data because drum-heavy music has flat, noisy chroma.
const MAJOR: [f64; 12] = [1.735, 0.666, 1.021, 0.637, 1.386, 0.817, 0.703, 1.659, 0.733, 0.855, 0.831, 0.956];
const MINOR: [f64; 12] = [1.881, 0.599, 1.024, 1.224, 0.698, 0.859, 0.574, 2.065, 0.654, 0.666, 1.080, 0.674];

/// Best-matching key and a 0..1 confidence (margin over the runner-up).
pub fn detect(ch: &Chroma) -> Option<(Key, f32)> {
    let total = total(ch);
    if total.iter().sum::<f64>() <= 1e-9 {
        return None;
    }
    let mut scores: Vec<(Key, f64)> = Vec::with_capacity(24);
    for (profile, minor) in [(&MAJOR, false), (&MINOR, true)] {
        for tonic in 0..12u8 {
            let rotated: Vec<f64> = (0..12).map(|i| profile[(i + 12 - usize::from(tonic)) % 12]).collect();
            scores.push((Key::new(tonic, minor), pearson(&total, &rotated)));
        }
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, s0) = scores[0];
    let s1 = scores[1].1;
    let conf = if s0 > 0.0 { ((s0 - s1) / s0 * 5.0).clamp(0.0, 1.0) } else { 0.0 };
    Some((best, conf as f32))
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        num += (x - ma) * (y - mb);
        da += (x - ma).powi(2);
        db += (y - mb).powi(2);
    }
    num / (da.sqrt() * db.sqrt()).max(1e-12)
}
