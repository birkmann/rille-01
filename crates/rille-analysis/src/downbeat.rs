//! Bar phase: which beat of four starts a bar.
//!
//! The arrangement changes at bar boundaries, most strongly at phrase starts
//! (every 8 or 16 bars: drops, breakdowns, elements entering or leaving).
//! Novelty at a beat compares the music just before it with the music just
//! after it over 4, 8 and 16 beats; windows of whole bars contain every beat
//! position once, so patterns inside the bar (claps on 2 and 4) cancel out and
//! only real changes remain. The phase whose beats carry the most novelty is
//! the downbeat.
//!
//! The clearest evidence are the big changes (a drop, a breakdown, the
//! kick leaving or coming back): they land on bar 1 nearly always, so they
//! vote on their own, and the average novelty breaks ties.

use crate::key::Chroma;

/// Per-beat band energies (log) and chroma, and the cues of the bar
/// position model (see [`beat_cues`]).
pub struct BeatFeatures {
    pub energy: Vec<[f64; 3]>,
    pub chroma: Vec<[f32; 12]>,
    pub cues: Vec<[f64; CUES]>,
}

/// Cues per beat for the bar position model.
pub const CUES: usize = 10;

/// The detected bar phase and how sure it is.
#[derive(Clone, Debug, Default)]
pub struct Downbeat {
    /// Beat index mod 4 that starts a bar.
    pub phase: i64,
    /// 0..1: how clearly the winner beats the runner-up.
    pub margin: f64,
    /// 0..1: share of the section-change evidence on the winning phase;
    /// 1 when there is none.
    pub agreement: f64,
    /// Number of clear section changes found.
    pub events: usize,
    /// 0..1: how much the section changes count (their total size).
    pub trust: f64,
    pub source: &'static str,
    /// Per phase (beat index mod 4): the section changes' share, and the
    /// average novelty scaled to 0..1.
    pub phrase_share: [f64; 4],
    pub novelty_share: [f64; 4],
    /// The bar position model's probability per phase.
    pub model: [f64; 4],
    /// With `RILLE_DOWNBEAT_DUMP` set: the novelty at every beat and the
    /// section changes' beat indices.
    pub dump: Option<(Vec<f64>, Vec<i64>)>,
}

/// Novelty at every beat: how different the music after it is from the
/// music before it, over 4, 8, 16 and 32 beats.
#[allow(clippy::needless_range_loop)] // parallel arrays indexed by beat
fn novelty_curve(features: &BeatFeatures) -> Vec<f64> {
    let n = features.energy.len();
    // Silence is not infinitely different from music: each band's log
    // energy is floored 26 dB (ln 6) below its loud level.
    let floor: [f64; 3] = std::array::from_fn(|k| {
        let mut v: Vec<f64> = features.energy.iter().map(|e| e[k]).collect();
        v.sort_by(f64::total_cmp);
        v.get(v.len() * 9 / 10).copied().unwrap_or(0.0) - 6.0
    });
    let energy: Vec<[f64; 3]> = features.energy.iter().map(|e| std::array::from_fn(|k| e[k].max(floor[k]))).collect();
    let chroma: Vec<[f32; 12]> = features
        .chroma
        .iter()
        .map(|c| {
            let norm = c.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
            c.map(|v| v / norm)
        })
        .collect();
    let mean = |a: usize, b: usize| -> ([f64; 3], [f32; 12]) {
        let mut e = [0.0f64; 3];
        let mut c = [0.0f32; 12];
        for i in a..b {
            for k in 0..3 {
                e[k] += energy[i][k];
            }
            for k in 0..12 {
                c[k] += chroma[i][k];
            }
        }
        let len = (b - a).max(1) as f64;
        (e.map(|v| v / len), c)
    };
    (0..n)
        .map(|i| {
            if i < 4 || i + 4 > n {
                return 0.0;
            }
            let mut novelty = 0.0;
            for w in [4usize, 8, 16, 32] {
                if i < w || i + w > n {
                    continue;
                }
                let (e0, c0) = mean(i - w, i);
                let (e1, c1) = mean(i, i + w);
                let de: f64 = (0..3).map(|k| (e1[k] - e0[k]).abs()).sum();
                novelty += de + 2.0 * (1.0 - cosine(&c0, &c1));
            }
            novelty
        })
        .collect()
}

/// Picks the bar phase. `first_idx` is the beat index of feature 0,
/// `first_strong_idx` the first beat with a clear attack.
///
/// Clear section changes (drops, breakdowns, elements entering) are peaks
/// of the novelty curve well above its usual level; each votes for its
/// phase by its size. The average novelty per phase and the first clear
/// beat add smaller votes and decide when there are no clear changes.
pub fn detect(features: &BeatFeatures, first_idx: i64, first_strong_idx: i64) -> Downbeat {
    let nv = novelty_curve(features);
    let n = nv.len();
    let phase_of = |i: usize| (first_idx + i as i64).rem_euclid(4) as usize;
    let mut mean = [0.0f64; 4];
    let mut count = [0u32; 4];
    for i in 4..n.saturating_sub(4) {
        mean[phase_of(i)] += nv[i];
        count[phase_of(i)] += 1;
    }
    let mean: [f64; 4] = std::array::from_fn(|p| mean[p] / f64::from(count[p].max(1)));

    let mut sorted: Vec<f64> = nv[4.min(n)..n.saturating_sub(4).max(4.min(n))].to_vec();
    sorted.sort_by(f64::total_cmp);
    let med = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let mut dev: Vec<f64> = sorted.iter().map(|v| (v - med).abs()).collect();
    dev.sort_by(f64::total_cmp);
    let mad = dev.get(dev.len() / 2).copied().unwrap_or(0.0).max(1e-9);
    // Clear changes: (beat index, size above the usual novelty).
    let mut found: Vec<(i64, f64)> = Vec::new();
    // Not near the ends: the windows are cut short there, and the music
    // starting or stopping is not a bar-level change.
    for i in 8..n.saturating_sub(8) {
        let local_max = (i.saturating_sub(2)..=(i + 2).min(n - 1)).all(|j| j == i || nv[j] < nv[i]);
        if local_max && nv[i] >= med + 6.0 * mad {
            found.push((first_idx + i as i64, nv[i] - med));
            if std::env::var_os("RILLE_DEBUG_DOWNBEAT").is_some() {
                eprintln!(
                    "event at beat {} (phase {}): {:.3} (median {med:.3}, mad {mad:.3})",
                    first_idx + i as i64,
                    phase_of(i),
                    nv[i]
                );
            }
        }
    }
    let events = found.len();
    let total: f64 = found.iter().map(|e| e.1).sum();
    // Big changes sit on a phrase lattice (every 4 bars) counted from the
    // first downbeat. Many tracks bring the kick back one beat early (a
    // pickup) or have a fill there, so an event also counts a quarter for
    // the lattice point one beat later. Each bar phase takes its best
    // phrase position: one consistent phrase structure outweighs pickups
    // scattered over other phases.
    let mut lattice = [0.0f64; 16];
    for &(beat, w) in &found {
        let q = beat.rem_euclid(16) as usize;
        lattice[q] += w;
        lattice[(q + 1) % 16] += 0.25 * w;
    }
    let phrase: [f64; 4] = std::array::from_fn(|p| (0..4).map(|k| lattice[p + 4 * k]).fold(0.0, f64::max));
    let phrase_total: f64 = phrase.iter().sum();
    // Share of the events on phase `p`, pickups one beat early counting half.
    let agreement_of = |p: usize| {
        let on = |d: i64| found.iter().filter(|e| (e.0 - p as i64 - d).rem_euclid(4) == 0).map(|e| e.1).sum::<f64>();
        if total > 0.0 { ((on(0) + 0.5 * on(-1)) / total).min(1.0) } else { 1.0 }
    };
    let (mmin, mmax) = mean.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let mean_share: [f64; 4] = std::array::from_fn(|p| (mean[p] - mmin) / (mmax - mmin).max(1e-12));
    // A few marginal peaks are not evidence: the events count fully once
    // their total size reaches eight times the typical novelty (two or
    // three real drops or breakdowns).
    let trust = (total / (8.0 * med.max(1e-9))).min(1.0);
    // What sounds like bar 1 (see `bar_model`), for loops without section
    // changes and pickups that blur them.
    let model = bar_model(features, &nv, first_idx);
    let mut score: [f64; 4] = std::array::from_fn(|p| {
        let ev = if phrase_total > 0.0 { phrase[p] / phrase_total } else { 0.0 };
        trust * ev + 0.3 * mean_share[p] + MODEL_WEIGHT * model[p]
    });
    score[first_strong_idx.rem_euclid(4) as usize] += 0.1;
    let mut order: Vec<usize> = (0..4).collect();
    order.sort_by(|&a, &b| score[b].total_cmp(&score[a]));
    let (best, second) = (score[order[0]], score[order[1]]);
    let phase = order[0];
    let agreement = agreement_of(phase);
    let margin = ((best - second) / best.max(1e-12)).clamp(0.0, 1.0);
    let source = if trust >= 0.5 && agreement >= 0.5 { "section changes" } else { "novelty" };
    let phrase_share = std::array::from_fn(|p| if phrase_total > 0.0 { phrase[p] / phrase_total } else { 0.0 });
    let dump = std::env::var_os("RILLE_DOWNBEAT_DUMP").map(|_| (nv.clone(), found.iter().map(|e| e.0).collect()));
    Downbeat {
        phase: phase as i64,
        margin,
        agreement,
        events,
        trust,
        source,
        phrase_share,
        novelty_share: mean_share,
        model,
        dump,
    }
}

/// Beats left out at each end by the bar position model.
const MODEL_EDGE: usize = 8;
/// Weight of the bar position model's probabilities in the phase score.
const MODEL_WEIGHT: f64 = 0.3;
/// Per beat: the [`CUES`], then the novelty, its upper decile and the share
/// of novelty peaks on the beat's phase (see [`bar_profile`]).
const PROFILE: usize = CUES + 3;

/// The bar position model: weights per bar position (beat 1..4 of a
/// candidate bar) and profile entry. Fitted with a softmax over the four
/// phases on the cues of tracks whose section changes agree on bar 1
/// (`rille-cli downbeat-dump`).
const BAR_MODEL: [[f64; PROFILE]; 4] = [
    [0.3694, 0.4193, 0.1139, 0.09172, 0.1054, -0.05819, 0.6414, -0.1275, -0.285, 0.6691, 0.9138, 0.952, 3.404],
    [
        -0.01554, -0.02984, -0.008471, 0.1272, 0.1099, 0.131, -0.4521, -0.1089, 0.01674, -0.03742, -0.3261, -0.5861,
        -1.024,
    ],
    [
        -0.1312, -0.125, -0.3367, -0.06323, 0.04255, -0.09164, -0.2871, 0.04552, 0.1292, -0.3953, -0.5885, -0.2231,
        -1.318,
    ],
    [-0.2226, -0.2644, 0.2312, -0.1556, -0.2579, 0.01884, 0.0978, 0.1908, 0.1391, -0.2364, 0.0008199, -0.1428, -1.061],
];

/// Per phase (beat index mod 4): the mean of each z-scored cue, and the
/// novelty's mean, upper decile (both z-scored) and share of local peaks,
/// over the beats away from the ends.
fn bar_profile(features: &BeatFeatures, nv: &[f64], first_idx: i64) -> Option<[[f64; PROFILE]; 4]> {
    let n = features.cues.len().min(nv.len());
    if n < 2 * MODEL_EDGE + 16 {
        return None;
    }
    let (lo, hi) = (MODEL_EDGE, n - MODEL_EDGE);
    let len = (hi - lo) as f64;
    let z_scored = |values: &mut dyn Iterator<Item = f64>| -> Vec<f64> {
        let v: Vec<f64> = values.collect();
        let mean = v.iter().sum::<f64>() / len;
        let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / len).sqrt();
        v.iter().map(|x| (x - mean) / (sd + 1e-6)).collect()
    };
    let cues: Vec<Vec<f64>> = (0..CUES).map(|c| z_scored(&mut (lo..hi).map(|i| features.cues[i][c]))).collect();
    let novelty = z_scored(&mut (lo..hi).map(|i| nv[i]));
    let peak = |k: usize| k > 0 && k + 1 < hi - lo && nv[lo + k] > nv[lo + k - 1] && nv[lo + k] >= nv[lo + k + 1];
    let phase = |k: usize| (first_idx + (lo + k) as i64).rem_euclid(4) as usize;
    let mut out = [[0.0; PROFILE]; 4];
    for (p, row) in out.iter_mut().enumerate() {
        let on: Vec<usize> = (0..hi - lo).filter(|&k| phase(k) == p).collect();
        let m = on.len().max(1) as f64;
        for c in 0..CUES {
            row[c] = on.iter().map(|&k| cues[c][k]).sum::<f64>() / m;
        }
        let mut nvs: Vec<f64> = on.iter().map(|&k| novelty[k]).collect();
        nvs.sort_by(f64::total_cmp);
        row[CUES] = nvs.iter().sum::<f64>() / m;
        row[CUES + 1] = percentile(&nvs, 0.9);
        row[CUES + 2] = on.iter().filter(|&&k| peak(k)).count() as f64 / m;
    }
    Some(out)
}

/// The `q` quantile of sorted `v`, interpolated between the nearest ranks.
fn percentile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let at = q * (v.len() - 1) as f64;
    let (i, frac) = (at.floor() as usize, at.fract());
    v[i] + frac * (v[(i + 1).min(v.len() - 1)] - v[i])
}

/// Probability of each phase (beat index mod 4) starting the bar, from what
/// the beats sound like (see [`BAR_MODEL`]); even without a profile.
fn bar_model(features: &BeatFeatures, nv: &[f64], first_idx: i64) -> [f64; 4] {
    let Some(prof) = bar_profile(features, nv, first_idx) else { return [0.25; 4] };
    let score: [f64; 4] = std::array::from_fn(|d| {
        (0..4).map(|k| BAR_MODEL[k].iter().zip(&prof[(d + k) % 4]).map(|(w, x)| w * x).sum::<f64>()).sum()
    });
    let top = score.iter().copied().fold(f64::MIN, f64::max);
    let e = score.map(|s| (s - top).exp());
    let total: f64 = e.iter().sum();
    e.map(|v| v / total)
}

fn cosine(a: &[f32; 12], b: &[f32; 12]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0.0f64, 0.0f64, 0.0f64);
    for k in 0..12 {
        ab += f64::from(a[k]) * f64::from(b[k]);
        aa += f64::from(a[k]).powi(2);
        bb += f64::from(b[k]).powi(2);
    }
    if aa <= 0.0 || bb <= 0.0 { 1.0 } else { ab / (aa.sqrt() * bb.sqrt()) }
}

/// Collects features for beats at `beat_secs` (consecutive indices).
pub fn features(bands: &[Vec<f32>; 3], sr: f64, chroma: &Chroma, beat_secs: &[f64]) -> BeatFeatures {
    let mut energy = Vec::with_capacity(beat_secs.len());
    let mut ch = Vec::with_capacity(beat_secs.len());
    // Log mean square of band `k` over `a..b` seconds.
    let level = |k: usize, a: f64, b: f64| {
        let (i0, i1) = ((a * sr).max(0.0) as usize, (b * sr).max(0.0) as usize);
        let seg = bands[k].get(i0.min(bands[k].len())..i1.min(bands[k].len())).unwrap_or(&[]);
        let ms = seg.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / seg.len().max(1) as f64;
        (ms + 1e-10).ln()
    };
    let mut attack = Vec::with_capacity(beat_secs.len());
    for w in beat_secs.windows(2) {
        let (a, b) = (w[0], w[1]);
        energy.push(std::array::from_fn(|k| level(k, a, b)));
        ch.push(chroma.mean(a, b));
        // The first sixth of the beat against the sixth before it.
        let d = (b - a) / 6.0;
        attack.push(std::array::from_fn::<f64, 3, _>(|k| level(k, a, a + d) - level(k, a - d, a)));
    }
    let cues = beat_cues(&energy, &ch, &attack);
    BeatFeatures { energy, chroma: ch, cues }
}

/// What starts at each beat, for telling bar 1 from the other beats inside
/// the bar: the attack in each band, the band energies and their change
/// from the beat before, and how much the harmony changes.
fn beat_cues(energy: &[[f64; 3]], chroma: &[[f32; 12]], attack: &[[f64; 3]]) -> Vec<[f64; CUES]> {
    (0..energy.len())
        .map(|i| {
            let prev = i.saturating_sub(1);
            let mut c = [0.0; CUES];
            for k in 0..3 {
                c[k] = attack[i][k];
                c[3 + k] = energy[i][k];
                c[6 + k] = energy[i][k] - energy[prev][k];
            }
            c[9] = if i == 0 { 0.0 } else { 1.0 - cosine(&chroma[prev], &chroma[i]) };
            c
        })
        .collect()
}
