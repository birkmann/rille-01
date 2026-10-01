//! Robust fitting of beat observations to a grid, and the constant /
//! piecewise / live classification.

/// One detected beat: integer beat index, time, and reliability weight.
#[derive(Clone, Copy, Debug)]
pub struct Obs {
    pub idx: i64,
    pub secs: f64,
    pub weight: f64,
}

/// Continuous piecewise-linear model `t(n) = c0 + c1·n + Σ dj·max(0, n − kj)`.
#[derive(Clone, Debug)]
pub struct Model {
    pub knots: Vec<f64>,
    pub coef: Vec<f64>,
}

impl Model {
    fn basis(knots: &[f64], n: f64) -> Vec<f64> {
        let mut v = vec![1.0, n];
        v.extend(knots.iter().map(|k| (n - k).max(0.0)));
        v
    }

    pub fn eval(&self, n: f64) -> f64 {
        Self::basis(&self.knots, n).iter().zip(&self.coef).map(|(b, c)| b * c).sum()
    }

    /// Seconds per beat in the segment containing `n`.
    pub fn slope_at(&self, n: f64) -> f64 {
        self.coef[1] + self.knots.iter().zip(&self.coef[2..]).filter(|(k, _)| n >= **k).map(|(_, d)| d).sum::<f64>()
    }
}

#[derive(Clone, Debug)]
pub struct Fit {
    pub model: Model,
    /// Residual per observation (seconds), same order as the input.
    pub residuals: Vec<f64>,
    /// Final robust weights (input weight × Huber weight).
    pub weights: Vec<f64>,
}

impl Fit {
    /// Weighted RMS residual in ms.
    pub fn rms_ms(&self) -> f64 {
        let (s, w) =
            self.residuals.iter().zip(&self.weights).fold((0.0, 0.0), |(s, sw), (r, w)| (s + w * r * r, sw + w));
        if w > 0.0 { (s / w).sqrt() * 1000.0 } else { f64::INFINITY }
    }

    /// Largest |weighted median residual| over sliding windows of `win`
    /// beats that contain enough reliable beats (ms). Measures systematic
    /// drift; per-beat jitter and single wrong beats don't move a median.
    /// Uses the input weights, so beats the robust fit rejected still count.
    pub fn max_drift_ms(&self, obs: &[Obs], win: i64) -> f64 {
        let mut worst: f64 = 0.0;
        let (first, last) = match (obs.first(), obs.last()) {
            (Some(f), Some(l)) => (f.idx, l.idx),
            _ => return 0.0,
        };
        let mut window: Vec<(f64, f64)> = Vec::new();
        let mut start = first;
        while start <= last {
            window.clear();
            window.extend(
                obs.iter()
                    .zip(&self.residuals)
                    .filter(|(o, _)| o.idx >= start && o.idx < start + win && o.weight >= STRONG)
                    .map(|(o, r)| (*r, o.weight)),
            );
            let total: f64 = window.iter().map(|w| w.1).sum();
            if total >= win as f64 * 0.5 {
                window.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut acc = 0.0;
                if let Some((r, _)) = window.iter().find(|(_, w)| {
                    acc += w;
                    acc >= total / 2.0
                }) {
                    worst = worst.max(r.abs());
                }
            }
            start += win / 4;
        }
        worst * 1000.0
    }

    /// Share of the strong beats' weight the robust fit kept.
    pub fn kept_fraction(&self, obs: &[Obs]) -> f64 {
        let strong = obs.iter().zip(&self.weights).filter(|(o, _)| o.weight >= STRONG);
        let (input, kept) = strong.fold((0.0, 0.0), |(i, k), (o, w)| (i + o.weight, k + w));
        if input > 0.0 { kept / input } else { 0.0 }
    }

    /// Robust cost comparable between models: input weights, residuals
    /// capped at the rejection distance.
    fn cost(&self, obs: &[Obs]) -> f64 {
        obs.iter().zip(&self.residuals).map(|(o, r)| o.weight * r.abs().min(REJECT_SECS).powi(2)).sum()
    }
}

/// Weighted least squares with Huber IRLS; residuals beyond `reject` get zero
/// weight. `obs` must be sorted by index.
pub fn robust_fit(obs: &[Obs], knots: &[f64], huber: f64, reject: f64) -> Option<Fit> {
    let k = 2 + knots.len();
    if obs.iter().filter(|o| o.weight > 0.0).count() < k + 2 {
        return None;
    }
    let mut robust = vec![1.0f64; obs.len()];
    let mut model = Model { knots: knots.to_vec(), coef: vec![0.0; k] };
    // A plain least-squares start can be bent by outliers so far that the
    // rejection step then throws away the true beats at both ends and fits
    // only the middle of the track. For straight lines, start from a
    // Theil–Sen estimate (median of pairwise slopes), which ignores up to
    // ~30 % outliers, and weight the first solve by distance from it.
    let mut first_iter = 0;
    if knots.is_empty()
        && let Some((a, b)) = theil_sen(obs)
    {
        for (o, r) in obs.iter().zip(robust.iter_mut()) {
            let res = (o.secs - (a + b * o.idx as f64)).abs();
            *r = if res > reject {
                0.0
            } else if res > huber {
                huber / res
            } else {
                1.0
            };
        }
        first_iter = 1;
    }
    for iter in first_iter..12 + first_iter {
        let mut ata = vec![vec![0.0f64; k]; k];
        let mut atb = vec![0.0f64; k];
        for (o, r) in obs.iter().zip(&robust) {
            let w = o.weight * r;
            if w <= 0.0 {
                continue;
            }
            let b = Model::basis(knots, o.idx as f64);
            for i in 0..k {
                atb[i] += w * b[i] * o.secs;
                for j in 0..k {
                    ata[i][j] += w * b[i] * b[j];
                }
            }
        }
        model.coef = solve(ata, atb)?;
        let mut changed = false;
        for (o, r) in obs.iter().zip(robust.iter_mut()) {
            let res = (o.secs - model.eval(o.idx as f64)).abs();
            let new = if res > reject && iter > 0 {
                0.0
            } else if res > huber {
                huber / res
            } else {
                1.0
            };
            changed |= (new - *r).abs() > 1e-6;
            *r = new;
        }
        if !changed && iter > 1 {
            break;
        }
    }
    let residuals = obs.iter().map(|o| o.secs - model.eval(o.idx as f64)).collect();
    let weights = obs.iter().zip(&robust).map(|(o, r)| o.weight * r).collect();
    Some(Fit { model, residuals, weights })
}

/// Size of the slow trend in a fit's residuals (ms): a robust degree-4
/// polynomial over the whole track, evaluated at its largest point.
/// Produced tracks deviate from a constant grid only locally (noisy attacks, a
/// section where another sound is detected), which a low-order curve ignores;
/// real tempo drift or tempo changes show up as a large trend.
pub fn trend_ms(obs: &[Obs], fit: &Fit) -> f64 {
    let strong: Vec<(f64, f64, f64)> = obs
        .iter()
        .zip(&fit.residuals)
        .filter(|(o, _)| o.weight >= STRONG)
        .map(|(o, r)| (o.idx as f64, *r, o.weight))
        .collect();
    if strong.len() < 32 {
        return 0.0;
    }
    let (lo, hi) = (strong[0].0, strong[strong.len() - 1].0);
    let x = |n: f64| 2.0 * (n - lo) / (hi - lo).max(1.0) - 1.0;
    const K: usize = 5;
    let basis = |n: f64| -> [f64; K] {
        let t = x(n);
        [1.0, t, t * t, t * t * t, t * t * t * t]
    };
    let mut robust = vec![1.0f64; strong.len()];
    let mut coef = vec![0.0f64; K];
    for _ in 0..8 {
        let mut ata = vec![vec![0.0f64; K]; K];
        let mut atb = vec![0.0f64; K];
        for ((n, r, w), rw) in strong.iter().zip(&robust) {
            let b = basis(*n);
            for i in 0..K {
                atb[i] += w * rw * b[i] * r;
                for j in 0..K {
                    ata[i][j] += w * rw * b[i] * b[j];
                }
            }
        }
        let Some(c) = solve(ata, atb) else { return 0.0 };
        coef = c;
        for ((n, r, _), rw) in strong.iter().zip(robust.iter_mut()) {
            let e = (r - basis(*n).iter().zip(&coef).map(|(b, c)| b * c).sum::<f64>()).abs();
            *rw = if e > HUBER_SECS * 2.0 { HUBER_SECS * 2.0 / e } else { 1.0 };
        }
    }
    (0..=100)
        .map(|i| {
            let n = lo + (hi - lo) * i as f64 / 100.0;
            basis(n).iter().zip(&coef).map(|(b, c)| b * c).sum::<f64>().abs()
        })
        .fold(0.0, f64::max)
        * 1000.0
}

/// Constant grids are kept unless the residual trend exceeds this (ms).
pub const MAX_TREND_MS: f64 = 30.0;

/// Median-of-slopes line through the reliable observations, using pairs at
/// least 32 beats apart (deterministic subsample for long tracks).
fn theil_sen(obs: &[Obs]) -> Option<(f64, f64)> {
    let strong: Vec<&Obs> = obs.iter().filter(|o| o.weight >= STRONG).collect();
    if strong.len() < 16 {
        return None;
    }
    let step = (strong.len() / 400).max(1);
    let pts: Vec<&Obs> = strong.iter().step_by(step).copied().collect();
    let mut slopes = Vec::with_capacity(pts.len() * pts.len() / 4);
    for i in 0..pts.len() {
        for j in i + 1..pts.len() {
            let dn = pts[j].idx - pts[i].idx;
            if dn >= 32 {
                slopes.push((pts[j].secs - pts[i].secs) / dn as f64);
            }
        }
    }
    if slopes.is_empty() {
        return None;
    }
    let mid = slopes.len() / 2;
    let b = *slopes.select_nth_unstable_by(mid, f64::total_cmp).1;
    let mut icpt: Vec<f64> = strong.iter().map(|o| o.secs - b * o.idx as f64).collect();
    let m = icpt.len() / 2;
    let a = *icpt.select_nth_unstable_by(m, f64::total_cmp).1;
    Some((a, b))
}

/// Gaussian elimination with partial pivoting.
#[allow(clippy::needless_range_loop)] // matrix code reads best with indices
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        x[r] = (b[r] - (r + 1..n).map(|k| a[r][k] * x[k]).sum::<f64>()) / a[r][r];
    }
    Some(x)
}

pub const HUBER_SECS: f64 = 0.003;
/// Beats at least this reliable count for drift checks.
pub const STRONG: f64 = 0.5;
pub const REJECT_SECS: f64 = 0.020;
/// A constant grid may deviate at most this much (median over 16 beats) from
/// the beats anywhere in the track. Produced tracks can show a few ms of
/// apparent drift where the kick sound changes; a constant grid still syncs
/// inaudibly there, while real tempo drift grows to tens of ms.
pub const MAX_DRIFT_MS: f64 = 12.0;
/// Piecewise grids must fit tighter than that to be worth their complexity.
const MAX_DRIFT_PIECEWISE_MS: f64 = 8.0;
const MIN_SEGMENT_BEATS: i64 = 32;
const MAX_KNOTS: usize = 4;

#[derive(Clone, Debug)]
pub enum Shape {
    Constant(Fit),
    Piecewise(Fit),
    /// One smoothed time per beat index from `first_idx`.
    Live {
        first_idx: i64,
        secs: Vec<f64>,
        fit: Fit,
    },
}

impl Shape {
    /// Every beat moved by `dt` seconds.
    pub fn shifted(mut self, dt: f64) -> Self {
        match &mut self {
            Shape::Constant(f) | Shape::Piecewise(f) => f.model.coef[0] += dt,
            Shape::Live { secs, fit, .. } => {
                secs.iter_mut().for_each(|t| *t += dt);
                fit.model.coef[0] += dt;
            }
        }
        self
    }

    pub fn fit(&self) -> &Fit {
        match self {
            Shape::Constant(f) | Shape::Piecewise(f) | Shape::Live { fit: f, .. } => f,
        }
    }

    /// Beat time for an index under this shape.
    pub fn secs_at(&self, n: i64) -> f64 {
        match self {
            Shape::Constant(f) | Shape::Piecewise(f) => f.model.eval(n as f64),
            Shape::Live { first_idx, secs, .. } => {
                let i = n - first_idx;
                if i < 0 {
                    secs[0] + i as f64 * (secs[1] - secs[0])
                } else if i as usize >= secs.len() {
                    let l = secs.len();
                    secs[l - 1] + (i as usize - (l - 1)) as f64 * (secs[l - 1] - secs[l - 2])
                } else {
                    secs[i as usize]
                }
            }
        }
    }
}

/// Picks the simplest model whose drift stays within [`MAX_DRIFT_MS`].
pub fn classify(obs: &[Obs]) -> Option<Shape> {
    let constant = robust_fit(obs, &[], HUBER_SECS, REJECT_SECS)?;
    let good = |f: &Fit, max: f64| f.max_drift_ms(obs, 16) <= max && f.kept_fraction(obs) >= 0.6;
    if good(&constant, MAX_DRIFT_MS) {
        return Some(Shape::Constant(constant));
    }

    // Greedily add tempo-change knots while each one clearly helps.
    let (first, last) = (obs.first()?.idx, obs.last()?.idx);
    let mut knots: Vec<f64> = Vec::new();
    let mut best = constant.clone();
    while knots.len() < MAX_KNOTS {
        let mut cand: Option<(f64, Fit)> = None;
        let mut k = first + MIN_SEGMENT_BEATS;
        while k <= last - MIN_SEGMENT_BEATS {
            let kf = k as f64;
            let spaced = knots.iter().all(|&e| (e - kf).abs() >= MIN_SEGMENT_BEATS as f64);
            if spaced {
                let mut trial = knots.clone();
                trial.push(kf);
                trial.sort_by(f64::total_cmp);
                if let Some(f) = robust_fit(obs, &trial, HUBER_SECS, REJECT_SECS)
                    && cand.as_ref().is_none_or(|(_, c)| f.cost(obs) < c.cost(obs))
                {
                    cand = Some((kf, f));
                }
            }
            k += 4;
        }
        let Some((kf, f)) = cand else { break };
        if f.cost(obs) > best.cost(obs) * 0.6 {
            break;
        }
        knots.push(kf);
        knots.sort_by(f64::total_cmp);
        best = f;
        if good(&best, MAX_DRIFT_PIECEWISE_MS) {
            return Some(Shape::Piecewise(best));
        }
    }

    Some(live(obs, &constant))
}

/// Per-beat map: local weighted linear regression over ±4 beats.
fn live(obs: &[Obs], fit: &Fit) -> Shape {
    let (first, last) = (obs[0].idx, obs[obs.len() - 1].idx);
    let mut secs = Vec::with_capacity((last - first + 1) as usize);
    let mut j0 = 0;
    for n in first..=last {
        while j0 < obs.len() && obs[j0].idx < n - 4 {
            j0 += 1;
        }
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for o in obs[j0..].iter().take_while(|o| o.idx <= n + 4) {
            let w = o.weight;
            let x = (o.idx - n) as f64;
            sw += w;
            sx += w * x;
            sy += w * o.secs;
            sxx += w * x * x;
            sxy += w * x * o.secs;
        }
        let det = sw * sxx - sx * sx;
        let t = if sw > 0.5 && det.abs() > 1e-9 { (sy * sxx - sx * sxy) / det } else { fit.model.eval(n as f64) };
        secs.push(t);
    }
    // Enforce strictly increasing positions.
    let min_gap = 0.2;
    for i in 1..secs.len() {
        if secs[i] < secs[i - 1] + min_gap {
            secs[i] = secs[i - 1] + min_gap.max(fit.model.coef[1]);
        }
    }
    Shape::Live { first_idx: first, secs, fit: fit.clone() }
}

/// BPM rounding steps, coarsest first.
pub const ALL_STEPS: [f64; 6] = [1.0, 0.5, 0.25, 0.1, 0.05, 0.01];

/// Rounder BPM values than `fit`'s (multiples of `steps`), roundest first,
/// whose grid stays within `max_drift` s of the fitted one end to end
/// (re-anchored in the middle).
pub fn round_tempos(obs: &[Obs], fit: &Fit, max_drift: f64, steps: &[f64]) -> Vec<(f64, Fit)> {
    let secs_per_beat = fit.model.coef[1];
    let bpm = 60.0 / secs_per_beat;
    let (Some(first), Some(last)) = (obs.first().map(|o| o.idx), obs.last().map(|o| o.idx)) else {
        return Vec::new();
    };
    let span_beats = (last - first).max(1) as f64;
    let mut out: Vec<(f64, Fit)> = Vec::new();
    for &step in steps {
        let snapped = (bpm / step).round() * step;
        let p = 60.0 / snapped;
        if (p - secs_per_beat).abs() * span_beats > max_drift || out.iter().any(|(b, _)| (b - snapped).abs() < 1e-9) {
            continue;
        }
        let mid = (first + last) as f64 / 2.0;
        let anchor = fit.model.eval(mid) - mid * p;
        let model = Model { knots: vec![], coef: vec![anchor, p] };
        let residuals: Vec<f64> = obs.iter().map(|o| o.secs - model.eval(o.idx as f64)).collect();
        out.push((snapped, Fit { model, residuals, weights: fit.weights.clone() }));
    }
    out
}

/// Tries rounder BPM values; accepts the first that moves the grid by at
/// most 1.5 ms anywhere in the track (3 ms end to end) and doesn't fit the
/// beats noticeably worse. Returns the snapped fit, or `None` to keep the
/// unsnapped value.
pub fn snap_constant(obs: &[Obs], fit: &Fit) -> Option<(f64, Fit)> {
    let base_rms = fit.rms_ms();
    round_tempos(obs, fit, 0.003, &ALL_STEPS)
        .into_iter()
        .find(|(_, cand)| cand.rms_ms() <= base_rms + (0.1 * base_rms).max(0.3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs_line(n: i64, a: f64, p: f64, noise: impl Fn(i64) -> f64) -> Vec<Obs> {
        (0..n).map(|i| Obs { idx: i, secs: a + i as f64 * p + noise(i), weight: 1.0 }).collect()
    }

    #[test]
    fn recovers_exact_line_with_outliers() {
        let p = 60.0 / 127.83;
        let mut obs = obs_line(600, 0.3712, p, |i| ((i * 7919) % 13) as f64 * 1e-4 - 6e-4);
        obs[100].secs += 0.05;
        obs[300].secs -= 0.1;
        let shape = classify(&obs).unwrap();
        let Shape::Constant(f) = shape else { panic!("expected constant") };
        assert!((f.model.coef[1] - p).abs() < 1e-6);
        assert!((f.model.coef[0] - 0.3712).abs() < 5e-4);
    }

    #[test]
    fn detects_tempo_change() {
        let (p1, p2) = (60.0 / 120.0, 60.0 / 126.0);
        let obs: Vec<Obs> = (0..400)
            .map(|i| {
                let t = if i < 200 { i as f64 * p1 } else { 200.0 * p1 + (i - 200) as f64 * p2 };
                Obs { idx: i, secs: t, weight: 1.0 }
            })
            .collect();
        match classify(&obs).unwrap() {
            Shape::Piecewise(f) => {
                assert!((f.model.slope_at(10.0) - p1).abs() < 1e-6);
                assert!((f.model.slope_at(390.0) - p2).abs() < 1e-6);
            }
            other => panic!("expected piecewise, got {other:?}"),
        }
    }

    #[test]
    fn snaps_only_when_harmless() {
        let exact = obs_line(700, 0.1, 60.0 / 128.0, |_| 0.0);
        let f = robust_fit(&exact, &[], HUBER_SECS, REJECT_SECS).unwrap();
        let (bpm, _) = snap_constant(&exact, &f).unwrap();
        assert_eq!(bpm, 128.0);
        // 125.97 must not be forced to 126: that would move the grid by
        // 0.03 BPM ≈ 80 ms over 700 beats.
        let odd = obs_line(700, 0.1, 60.0 / 125.97, |_| 0.0);
        let f = robust_fit(&odd, &[], HUBER_SECS, REJECT_SECS).unwrap();
        let snapped = snap_constant(&odd, &f).map(|s| s.0);
        assert!(snapped.is_none_or(|b| (b - 125.97).abs() < 0.006), "{snapped:?}");
    }
}
