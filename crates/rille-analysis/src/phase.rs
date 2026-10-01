//! Where exactly the beat starts, from the average of all beats.
//!
//! Per-beat attack times are noisy in dense mixes, and "the kick band rises"
//! is not always the kick: a sidechained bassline swells up tens of ms after
//! each kick. Averaging the envelope around hundreds of grid lines removes
//! the noise and shows the typical beat event, the same picture as
//! `rille-cli gridplot`. Both envelopes come from causal filters, so nothing
//! rises before the sound does.
//!
//! A kick rises sharply in the kick band *and* in the broadband at the same
//! moment (its body and its click or pitch sweep). A sidechained bassline
//! swelling back up rises only in the kick band, and slowly; a hat rises only
//! in the broadband. So the beat is the earliest moment where both bands rise
//! together (halfway points within 8 ms), timed by the sharper of the two
//! edges. Without such a moment, the strongest sharp kick-band rise, then the
//! first broadband attack, are used.

use crate::refine::TransientFinder;

const HALF: f64 = 0.06;
const STEP: f64 = 0.00025;

/// Running maximum over the last `hold` seconds: the energy of a low tone
/// swings from zero to full every half period, identically in every beat, so
/// averaging alone keeps the oscillation. The held peak follows the attack's
/// growth without it.
fn peak_hold(row: &[f32], hold: f64) -> Vec<f32> {
    let w = (hold / STEP).round().max(1.0) as usize;
    (0..row.len()).map(|i| row[i.saturating_sub(w)..=i].iter().copied().fold(0.0, f32::max)).collect()
}

/// Average of per-beat peak-held envelopes (each normalised to its maximum,
/// beats weighted by loudness so breakdowns don't dilute the profile).
fn profile(env: &dyn Fn(f64) -> Vec<f32>, hold: f64, lines: &[f64]) -> Vec<f64> {
    let n = (2.0 * HALF / STEP) as usize + 1;
    let rows: Vec<Vec<f32>> = lines.iter().map(|&g| peak_hold(&env(g), hold)).collect();
    let mut peaks: Vec<f32> = rows.iter().map(|r| r.iter().copied().fold(0.0, f32::max)).collect();
    peaks.sort_by(f32::total_cmp);
    let med = peaks.get(peaks.len() / 2).copied().unwrap_or(0.0).max(1e-20);
    let mut acc = vec![0.0f64; n];
    for r in &rows {
        let max = r.iter().copied().fold(1e-20f32, f32::max);
        let w = f64::from((max / med).min(1.0));
        for (a, v) in acc.iter_mut().zip(r) {
            *a += w * f64::from(v / max);
        }
    }
    acc
}

/// A sharp rise in a profile: where it starts (s relative to the line), its
/// size (linear, in units of the normalised profile) and speed (10→90 %).
#[derive(Clone, Copy, Debug)]
pub struct Rise {
    pub start: f64,
    /// Where it is halfway up.
    pub mid: f64,
    pub size: f64,
    pub rise_ms: f64,
}

/// Rises within ±45 ms of the line that are at least 20 % the size of the
/// biggest one and reach 90 % within `max_rise_ms`, earliest first.
fn rises(p: &[f64], max_rise_ms: f64) -> Vec<Rise> {
    let n = p.len();
    let at = |i: usize| -HALF + i as f64 * STEP;
    let pts = |ms: f64| (ms / 1000.0 / STEP).round() as usize;
    let span = pts(1.0);
    let slope = |i: usize| p[(i + span).min(n - 1)] - p[i.saturating_sub(span)];
    let (w0, w1) = (pts(15.0), n - pts(15.0));
    let mut all: Vec<Rise> = Vec::new();
    let mut i = w0;
    while i < w1 {
        if slope(i) > 0.0 && slope(i) >= slope(i - 1) && slope(i) >= slope(i + 1) {
            let lo = i.saturating_sub(pts(15.0));
            let hi = (i + pts(15.0)).min(n - 1);
            let min_at = (lo..=i).min_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap_or(lo);
            let floor = p[min_at];
            let top_at = (i..=hi).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap_or(hi);
            let top = p[top_at];
            let (t10, t90) = (floor + 0.1 * (top - floor), floor + 0.9 * (top - floor));
            let s = (min_at..=top_at).find(|&k| p[k] >= t10).unwrap_or(i);
            let e = (s..=top_at).find(|&k| p[k] >= t90).unwrap_or(top_at);
            let m = (s..=top_at).find(|&k| p[k] >= 0.5 * (floor + top)).unwrap_or(i);
            let frac = if s > 0 && p[s] > p[s - 1] { (t10 - p[s - 1]) / (p[s] - p[s - 1]) } else { 1.0 };
            all.push(Rise {
                start: at(s) - STEP * (1.0 - frac.clamp(0.0, 1.0)),
                mid: at(m),
                size: top - floor,
                rise_ms: (e - s) as f64 * STEP * 1000.0,
            });
            i = top_at.max(i + 1);
            continue;
        }
        i += 1;
    }
    let biggest = all.iter().map(|r| r.size).fold(0.0, f64::max);
    let mut out: Vec<Rise> = all.into_iter().filter(|r| r.size >= 0.2 * biggest && r.rise_ms <= max_rise_ms).collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out.dedup_by(|b, a| b.start - a.start < 0.003);
    out
}

/// What the correction found, for reports and debugging.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhaseFix {
    /// Seconds to add to every grid line.
    pub shift: f64,
    pub source: &'static str,
}

/// How far the grid lines at `lines` are from the start of the beat event.
/// `None` if no clear event is found (the grid stays as it is).
pub fn beat_onset(low: &TransientFinder, broad: &TransientFinder, lines: &[f64]) -> Option<PhaseFix> {
    if lines.len() < 16 {
        return None;
    }
    let n = (2.0 * HALF / STEP) as usize + 1;
    let pb = profile(&|g| broad.start_envelope(g, HALF, n), 0.006, lines);
    let pl = profile(&|g| low.start_envelope(g, HALF, n), 0.016, lines);
    if std::env::var_os("RILLE_DEBUG_PROFILE").is_some() {
        let show = |name: &str, p: &[f64]| {
            let vals: Vec<String> = p.iter().step_by(8).map(|v| format!("{v:.0}")).collect();
            eprintln!("{name} (2 ms steps from -60 ms): {}", vals.join(" "));
        };
        show("broad", &pb);
        show("low", &pl);
    }
    // Only rises the fix may move to: the lines already sit on the kick
    // (see `kick`), and a pair further out would only be rejected below.
    let within = |v: Vec<Rise>| v.into_iter().filter(|r| r.start.abs() <= MAX_SHIFT).collect::<Vec<_>>();
    let broad_rises = within(rises(&pb, 25.0));
    let low_rises = within(rises(&pl, 25.0));
    if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
        eprintln!("phase: broad rises {broad_rises:?}");
        eprintln!("phase: low rises {low_rises:?}");
    }
    // Kicks whose body swells slowly (hard techno, hardstyle) start with a
    // click and reach full level tens of ms later: the kick-band rise then
    // starts shortly after the broadband one instead of sharing its midpoint.
    let together = broad_rises.iter().find_map(|b| {
        low_rises
            .iter()
            .find(|l| {
                (l.mid - b.mid).abs() <= 0.008
                    || (l.rise_ms > b.rise_ms && (-0.002..=0.012).contains(&(l.start - b.start)))
            })
            .map(|l| if l.rise_ms < b.rise_ms { l.start } else { b.start })
    });
    let fix = if let Some(t) = together {
        PhaseFix { shift: t, source: "kick (both bands)" }
    } else if let Some(l) = low_rises.iter().filter(|r| r.rise_ms <= 12.0).max_by(|a, b| a.size.total_cmp(&b.size)) {
        PhaseFix { shift: l.start, source: "kick band" }
    } else {
        // The biggest sharp broadband attack (a slow swell or a small tick
        // just before the kick is not it); the earliest if none is sharp.
        let sharp = broad_rises.iter().filter(|r| r.rise_ms <= 12.0).max_by(|a, b| a.size.total_cmp(&b.size));
        PhaseFix { shift: sharp.or(broad_rises.first())?.start, source: "broadband attack" }
    };
    (fix.shift.abs() <= MAX_SHIFT).then_some(fix)
}

/// Largest phase correction: the grid already sits on the kick (see `kick`).
const MAX_SHIFT: f64 = 0.045;

/// How far a beat may sit from its grid line and still be aligned.
const MAX_LAG: f64 = 0.02;

/// Onset curve of one beat around `g`: the rising part of the peak-held
/// envelope (normalised to the beat's maximum), both bands side by side.
/// Covers ±(`HALF` + `MAX_LAG`) so it can be shifted against the template.
fn onset_row(low: &TransientFinder, broad: &TransientFinder, g: f64) -> Vec<f32> {
    let half = HALF + MAX_LAG;
    let n = (2.0 * half / STEP) as usize + 1;
    let d = (0.001 / STEP) as usize;
    let mut out = Vec::with_capacity(2 * n);
    for (env, hold) in [(low.start_envelope(g, half, n), 0.016), (broad.start_envelope(g, half, n), 0.006)] {
        let held = peak_hold(&env, hold);
        let max = held.iter().copied().fold(1e-20f32, f32::max);
        out.extend((0..n).map(|i| (held[i] - held[i.saturating_sub(d)]).max(0.0) / max));
    }
    out
}

/// Every beat's position from matching its onset curve against the average
/// beat of the track (template matching): far steadier than timing each
/// beat's attack on its own, and the same for every kind of kick. `lines`
/// are (beat index, grid time); the result has one observation per beat
/// whose match is clear, weighted by how well it matches (0–1).
pub fn align_beats(low: &TransientFinder, broad: &TransientFinder, lines: &[(i64, f64)]) -> Vec<crate::fit::Obs> {
    if lines.len() < 16 {
        return Vec::new();
    }
    let rows: Vec<Vec<f32>> = lines.iter().map(|&(_, g)| onset_row(low, broad, g)).collect();
    let n = rows[0].len() / 2;
    let lag_pts = (MAX_LAG / STEP).round() as usize;
    let inner = n - 2 * lag_pts;
    // Template: the average onset curve over the central ±HALF, louder
    // beats counting fully and quiet ones less.
    let mut tpl = vec![0.0f64; 2 * inner];
    for r in &rows {
        for band in 0..2 {
            for i in 0..inner {
                tpl[band * inner + i] += f64::from(r[band * n + lag_pts + i]);
            }
        }
    }
    let tn = tpl.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-20);
    tpl.iter_mut().for_each(|v| *v /= tn);
    // Normalised correlation of a row with the template, the row shifted
    // by `lag` points (positive = the beat comes later than the line).
    let corr = |r: &[f32], lag: isize| -> f64 {
        let (mut dot, mut rr) = (0.0f64, 0.0f64);
        for band in 0..2 {
            let off = (band * n) as isize + lag_pts as isize + lag;
            for i in 0..inner {
                let v = f64::from(r[(off + i as isize) as usize]);
                dot += v * tpl[band * inner + i];
                rr += v * v;
            }
        }
        if rr > 0.0 { dot / rr.sqrt() } else { 0.0 }
    };
    let coarse = (0.001 / STEP) as isize;
    let lp = lag_pts as isize;
    lines
        .iter()
        .zip(&rows)
        .filter_map(|(&(idx, g), r)| {
            let mut best = (0isize, f64::MIN);
            for lag in (-lp..=lp).step_by(coarse as usize) {
                let c = corr(r, lag);
                if c > best.1 {
                    best = (lag, c);
                }
            }
            let centre = best.0;
            for lag in (centre - coarse).max(-lp)..=(centre + coarse).min(lp) {
                let c = corr(r, lag);
                if c > best.1 {
                    best = (lag, c);
                }
            }
            let (lag, c) = best;
            // Sub-step peak by a parabola through the neighbours.
            let frac = if lag > -lp && lag < lp {
                let (a, b) = (corr(r, lag - 1), corr(r, lag + 1));
                let den = a - 2.0 * c + b;
                if den < 0.0 { (0.5 * (a - b) / den).clamp(-0.5, 0.5) } else { 0.0 }
            } else {
                0.0
            };
            (c >= 0.3 && lag.abs() < lp).then(|| crate::fit::Obs {
                idx,
                secs: g + (lag as f64 + frac) * STEP,
                weight: c.min(1.0),
            })
        })
        .collect()
}
