//! Where the kick sits within the beat.
//!
//! The tempo lattice folds kick-band spectral flux, which in log units rewards
//! any sound rising out of silence: a sidechained bassline swelling back up,
//! a sub note on the off-beat, or a quiet tick in front of the kick can all
//! out-stack the kick itself. The later stages refine within tens of ms and
//! can drift onto a neighbouring sound. Either way the grid ends up a
//! sixteenth or an eighth off, or tens of ms early.
//!
//! A kick hits: the broadband jumps (its click or pitch sweep) and at the
//! same moment its low end arrives. A hat or clap clicks without low end; a
//! bass swells in without a click. The whole track is folded at the tempo
//! (the mean over every beat, which averages away per-beat noise) and each
//! phase of the beat is scored by `click × √punch`:
//! - *click*: the sharpest broadband step there, in dB.
//! - *punch*: low-band power gained within 6 ms, as a share of the loudest
//!   point of the beat. Linear power, so a quiet sound rising out of silence
//!   counts as little as it is loud; the square root keeps a loud bass note
//!   without a click from outweighing a kick with a smaller low end.

use crate::filter::{Biquad, filtfilt};

/// Profile resolution (s).
const STEP: f64 = 0.001;

#[derive(Clone, Copy, Debug)]
pub struct KickPhase {
    /// A time at which a kick starts (any beat; only the phase matters).
    pub secs: f64,
    /// Kick start relative to the nearest anchor line, in (−½, ½] beat (s).
    pub offset: f64,
    /// Low-band power gained within 6 ms, as a share of the beat's peak.
    pub punch: f64,
    /// Sharpest broadband step at the kick (dB).
    pub click_db: f64,
    pub score: f64,
    /// Best score within ±5 ms of the anchor the fold was made around.
    pub score_at_anchor: f64,
    /// Best score within ±15 ms of the anchor, for anchors that are only
    /// roughly placed (the tempo lattice's bins are ~7 ms wide).
    pub score_near_anchor: f64,
    /// Best score within ±12 ms of half a beat away from the kick.
    pub score_at_half: f64,
}

impl KickPhase {
    /// A clear kick: a sharp click with real low end under it. Under a
    /// heavy sub bass the kick's own share of the low end can be small, so a
    /// strong click needs less of it.
    pub fn clear(&self) -> bool {
        self.click_db >= 4.0 && self.punch >= 0.05 || self.click_db >= 8.0 && self.punch >= 0.02
    }

    /// The anchor is clearly not on the kick, which is elsewhere in the beat.
    /// A `rough` anchor only needs to be near it.
    pub fn anchor_misses(&self, rough: bool) -> bool {
        let (at, tolerance) = if rough { (self.score_near_anchor, 0.015) } else { (self.score_at_anchor, 0.010) };
        self.clear() && self.score > 1.3 * at && self.offset.abs() > tolerance
    }
}

/// Kick-band and broadband envelopes of a track, at 1 ms resolution.
pub struct KickFold {
    low: Vec<f32>,
    broad: Vec<f32>,
}

impl KickFold {
    /// Works at the full sample rate: much of a kick's click lies above what
    /// the analysis' decimated bands keep.
    pub fn new(mono: &[f32], sr: f64) -> Self {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let mut x = mono.to_vec();
        filtfilt(&[Biquad::highpass(sr, 30.0, q), Biquad::lowpass(sr, 200.0, q)], &mut x);
        // Low band power as x² + (x'/ω)², which follows the envelope of a
        // low tone without the ripple of x² alone.
        let w = (2.0 * std::f64::consts::PI * 60.0 / sr) as f32;
        let low = per_step(
            |i| {
                let d = if i > 0 && i + 1 < x.len() { (x[i + 1] - x[i - 1]) * 0.5 } else { 0.0 };
                x[i] * x[i] + (d / w).powi(2)
            },
            x.len(),
            sr,
        );
        x.copy_from_slice(mono);
        filtfilt(&[Biquad::highpass(sr, 200.0, q)], &mut x);
        let broad = per_step(|i| x[i] * x[i], x.len(), sr);
        Self { low, broad }
    }

    /// The kick phase at `bpm`, folding with bin 0 at `anchor`. `None` if
    /// the track is too short.
    pub fn find(&self, bpm: f64, anchor: f64) -> Option<KickPhase> {
        self.find_in(bpm, anchor, (0.0, f64::MAX))
    }

    /// Kick offsets from a constant grid (`bpm`, a line at `anchor`) in each
    /// of `parts` equal parts of the track where the kick is clear. A
    /// constant tempo keeps them together; a drifting or changing one spreads
    /// them over tens of ms.
    pub fn section_offsets(&self, bpm: f64, anchor: f64, parts: usize) -> Vec<f64> {
        let len = self.low.len() as f64 * STEP / parts as f64;
        (0..parts)
            .filter_map(|k| self.find_in(bpm, anchor, (k as f64 * len, (k + 1) as f64 * len)))
            .filter(|k| k.clear())
            .map(|k| k.offset)
            .collect()
    }

    fn find_in(&self, bpm: f64, anchor: f64, span: (f64, f64)) -> Option<KickPhase> {
        let n = (60.0 / bpm / STEP).round() as usize;
        let beats_in = (span.1.min(self.low.len() as f64 * STEP) - span.0) * bpm / 60.0;
        if n < 200 || beats_in < 16.0 {
            return None;
        }
        let fold = |env: &[f32], f: &dyn Fn(f64) -> f64| fold(env, bpm, anchor, span, f);
        let lin = fold(&self.low, &|v| v);
        // Broadband in dB; silence is floored 40 dB below the mean.
        let mean = self.broad.iter().map(|v| f64::from(*v)).sum::<f64>() / self.broad.len().max(1) as f64;
        let floor = mean * 1e-4 + 1e-20;
        let br = fold(&self.broad, &|v| 10.0 * (v + floor).log10());
        let peak = lin.iter().copied().fold(1e-20, f64::max);
        let at = |p: &[f64], i: isize| p[i.rem_euclid(n as isize) as usize];
        let mean = |p: &[f64], a: isize, b: isize| (a..=b).map(|i| at(p, i)).sum::<f64>() / (b - a + 1) as f64;
        let punch = |i: isize| (mean(&lin, i + 1, i + 6) - mean(&lin, i - 10, i - 3)) / peak;
        // The sharpest 4-ms-vs-4-ms broadband step near the phase, and where.
        let step = |j: isize| mean(&br, j, j + 3) - mean(&br, j - 8, j - 4);
        let click =
            |i: isize| (i - 3..=i + 6).map(|j| (j, step(j))).fold((i, f64::MIN), |m, x| if x.1 > m.1 { x } else { m });
        let scored: Vec<(f64, f64, f64, isize)> = (0..n as isize)
            .map(|i| {
                let (p, (cj, c)) = (punch(i), click(i));
                (p, c, c.max(0.0) * p.max(0.0).sqrt(), cj)
            })
            .collect();
        let best = (0..n).max_by(|&a, &b| scored[a].2.total_cmp(&scored[b].2))?;
        let near = |c: usize, w: usize| (0..=2 * w).map(|k| scored[(c + n - w + k) % n].2).fold(0.0, f64::max);
        let (punch, click_db, score, click_at) = scored[best];
        let period = 60.0 / bpm;
        // The click marks the start.
        let offset = wrap(click_at as f64 * period / n as f64, period);
        let kick = KickPhase {
            secs: anchor + offset,
            offset,
            punch,
            click_db,
            score,
            score_at_anchor: near(0, 5),
            score_near_anchor: near(0, 15),
            score_at_half: near((best + n / 2) % n, 12),
        };
        if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
            eprintln!("kick phase: {:+.1} ms from the anchor, {kick:?}", offset * 1000.0);
        }
        Some(kick)
    }
}

/// Mean of `power` per `STEP` with exact bin bounds (whole-sample chunks
/// would drift over a track).
fn per_step(power: impl Fn(usize) -> f32, len: usize, sr: f64) -> Vec<f32> {
    let per = STEP * sr;
    let n = (len as f64 / per) as usize;
    (0..n)
        .map(|k| {
            let (a, b) = ((k as f64 * per) as usize, (((k + 1) as f64 * per) as usize).min(len));
            (a..b).map(&power).sum::<f32>() / (b - a).max(1) as f32
        })
        .collect()
}

/// Mean over every beat starting within `span` (s) of `f(envelope value)`
/// at each phase bin, folded at `bpm` with bin 0 at `anchor`.
fn fold(env: &[f32], bpm: f64, anchor: f64, span: (f64, f64), f: &dyn Fn(f64) -> f64) -> Vec<f64> {
    let period = 60.0 / bpm;
    let n = (period / STEP).round() as usize;
    let first = -(anchor / period).floor() as i64;
    let last = ((env.len() as f64 * STEP - anchor) / period).floor() as i64 - 1;
    let mut acc = vec![0.0f64; n];
    let mut beats = 0.0;
    for k in first..=last {
        let t0 = anchor + k as f64 * period;
        let i0 = (t0 / STEP).round();
        if t0 < span.0 || t0 >= span.1 || i0 < 0.0 || i0 as usize + n + 1 >= env.len() {
            continue;
        }
        for (j, a) in acc.iter_mut().enumerate() {
            let i = ((t0 + j as f64 * period / n as f64) / STEP).round() as usize;
            *a += f(f64::from(env[i]));
        }
        beats += 1.0;
    }
    if beats > 0.0 {
        acc.iter_mut().for_each(|a| *a /= beats);
    }
    acc
}

/// `dt` wrapped into (−period/2, period/2].
pub fn wrap(dt: f64, period: f64) -> f64 {
    dt - (dt / period).round() * period
}
