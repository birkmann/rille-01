//! Beatgrid detection: ties tempo estimation, beat tracking, transient
//! refinement, fitting, snapping and downbeat detection together.

use rille_core::beatgrid::{BeatMap, ConstantMap, LiveMap, PiecewiseMap, TempoMarker};
use rille_core::{BeatClock, BeatGrid, GridFlags, GridSource, track::ANALYZER_VERSION};

use crate::fit::{self, Obs, Shape};
use crate::refine::{Band, TransientFinder};
use crate::{downbeat, key::Chroma, kick, onset, tempo, tracker};

/// Numbers describing how well the grid fits; printed by `rille-cli analyze`.
#[derive(Clone, Debug, Default)]
pub struct GridReport {
    pub kind: &'static str,
    /// What set the exact phase ("broadband attack" or "kick band").
    pub phase_source: &'static str,
    pub band: &'static str,
    /// Tempo of the unsnapped fit (first segment for piecewise grids).
    pub bpm_fit: f64,
    pub bpm: f64,
    /// Weighted RMS distance of detected beats from the grid (ms).
    pub rms_ms: f64,
    /// Largest systematic deviation over 16-beat windows (ms).
    pub drift_ms: f64,
    /// Share of strong transients within 5 ms of a grid line.
    pub hit_rate: f64,
    pub hit_p50_ms: f64,
    pub hit_p95_ms: f64,
    pub strong_beats: usize,
    pub octave_margin: f64,
    pub downbeat_margin: f64,
    /// Share of the kick evidence that agrees with the chosen downbeat.
    pub downbeat_agreement: f64,
    pub downbeat_source: &'static str,
    /// Clear section changes that voted on the downbeat.
    pub downbeat_events: usize,
    /// With `RILLE_DOWNBEAT_DUMP` set: the bar phase evidence and per-beat
    /// cues as JSON, for fitting the bar position model.
    pub downbeat_dump: String,
}

/// Low-rate signals shared with key detection and downbeat features.
pub struct Decimated {
    pub sr: f64,
    /// Low / mid / high bands.
    pub bands: [Vec<f32>; 3],
    pub chroma: Chroma,
}

pub fn detect(mono: &[f32], sr: f64, dec: &Decimated, bpm_range: (f64, f64)) -> (Option<BeatGrid>, GridReport) {
    let mut report = GridReport::default();
    let timer = Timer::new();
    let onsets = onset::compute(mono, sr as u32);
    timer.lap("onsets");
    let Some(tempo) = tempo::estimate(&onsets.combined, onsets.fps, bpm_range) else {
        report.kind = "none";
        return (None, report);
    };
    report.octave_margin = tempo.octave_margin;
    let duration = mono.len() as f64 / sr;

    // The constant-tempo lattice that stacks the kick-band onsets best.
    let (lat_bpm, mut lat_phase, _) = tempo::lattice(&onsets.low, onsets.fps, tempo.bpm);
    // Its phase can sit on a bassline or a hat instead of the kick (see
    // `kick`); later stages only search tens of ms around it, so put it on
    // the kick here. When the kick is clear and the half-beat position is
    // clearly not one, the half-beat corrections below must not undo it.
    let period_secs = 60.0 / lat_bpm;
    let kick_fold = kick::KickFold::new(mono, sr);
    let mut on_kick = false;
    if let Some(k) = kick_fold.find(lat_bpm, lat_phase) {
        let moved = k.anchor_misses(true);
        if moved {
            lat_phase = k.secs;
        }
        on_kick = (moved || k.clear() && k.offset.abs() < 0.015) && k.score_at_half < 0.5 * k.score;
    }
    let lattice_line = |n: i64| lat_phase + n as f64 * 60.0 / lat_bpm;
    timer.lap("tempo+lattice");

    // Attacks at every lattice beat, per band; keep the band whose attacks
    // sit most consistently on a grid (kick band unless it has no clear kick).
    // The kick sets the grid whenever there is one: other sounds near the
    // beat (claps, hats, click layers) must never move the lines off it.
    // Broadband attacks are used only for tracks without a kick.
    let mut chosen = band_fit(TransientFinder::new(mono, sr, Band::Low), &lattice_line, duration, !on_kick);
    if std::env::var_os("RILLE_DEBUG_OBS").is_some()
        && let Some(bf) = &chosen
    {
        eprintln!("band Low: attacks on {:.3} of beats, consistency {:.3}", bf.attack_share, bf.consistency);
    }
    if chosen.as_ref().is_none_or(|bf| bf.attack_share < 0.4) {
        chosen = band_fit(TransientFinder::new(mono, sr, Band::Broad), &lattice_line, duration, !on_kick).or(chosen);
    }
    let Some(BandFit { finder, med, obs: lattice_obs, fit: lattice_fit, .. }) = chosen else {
        report.kind = "none";
        return (None, report);
    };
    report.band = if finder.band() == Band::Low { "low" } else { "broad" };
    debug_obs("lattice", &lattice_obs);
    timer.lap("bands");

    // Candidate grids: constant, the same beats fitted as piecewise/live, and
    // the beat tracker's view for tracks whose tempo really changes.
    // Beats as the tracker sees them (it follows tempo changes, which the
    // lattice cannot).
    let period = 60.0 * onsets.fps / tempo.bpm;
    let mut frames = tracker::track(&onsets.combined, period, 400.0);
    let half = (period / 2.0).round() as usize;
    let low_at = |f: usize| f64::from(onsets.low.get(f).copied().unwrap_or(0.0));
    let on: f64 = frames.iter().map(|&f| low_at(f)).sum();
    let off: f64 = frames.iter().map(|&f| low_at(f + half)).sum();
    let tracker_off_kick = if on_kick {
        // On the kick's phase, not on whichever half has more kick-band flux.
        let mut d: Vec<f64> =
            frames.iter().map(|&f| kick::wrap(f as f64 / onsets.fps - lat_phase, period_secs).abs()).collect();
        d.sort_by(f64::total_cmp);
        d.get(d.len() / 2).is_some_and(|m| *m > period_secs / 4.0)
    } else {
        off > on * 1.3
    };
    if tracker_off_kick {
        frames.iter_mut().for_each(|f| *f += half);
    }
    let mut found: Vec<(f64, f64)> = frames
        .iter()
        .filter_map(|&f| finder.attack(f as f64 / onsets.fps, 0.035))
        .map(|t| (t.secs, norm(t.strength, med)))
        .collect();
    found.dedup_by(|b, a| (b.0 - a.0).abs() < 0.05);
    let tracked = assign_indices(&found, 60.0 / tempo.bpm);

    // Only a large, slow trend in a constant grid's residuals is evidence of
    // a changing tempo; otherwise flexible grids would just follow noise.
    let trend_lattice = fit::trend_ms(&lattice_obs, &lattice_fit);
    let trend_tracked =
        fit::robust_fit(&tracked, &[], fit::HUBER_SECS, fit::REJECT_SECS).map_or(0.0, |f| fit::trend_ms(&tracked, &f));
    if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
        eprintln!("residual trend: lattice {trend_lattice:.1} ms, tracker {trend_tracked:.1} ms");
    }
    // The kick folded per eighth of the track at the lattice tempo: if it
    // sits at the same phase everywhere, the tempo is constant, however noisy
    // the single attacks are (a busy mix, a swung or distorted kick).
    let sections = kick_fold.section_offsets(lat_bpm, lat_phase, 8);
    let spread = sections.iter().fold(f64::MIN, |m, v| m.max(*v)) - sections.iter().fold(f64::MAX, |m, v| m.min(*v));
    // Most sections must show it: at a tempo that fits only part of the
    // track, the kick smears (unclear) everywhere else.
    let kick_steady = sections.len() >= 6 && spread <= 0.010;
    if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
        let ms: Vec<String> = sections.iter().map(|o| format!("{:+.1}", o * 1000.0)).collect();
        eprintln!("kick per section (ms): {}", ms.join(" "));
    }
    let tempo_varies = trend_lattice.max(trend_tracked) > fit::MAX_TREND_MS && !kick_steady;
    let mut candidates: Vec<(Shape, Vec<Obs>)> = vec![(Shape::Constant(lattice_fit), lattice_obs.clone())];
    if tempo_varies
        && let Some(shape) = fit::classify(&lattice_obs)
        && !matches!(shape, Shape::Constant(_))
    {
        candidates.push((shape, lattice_obs.clone()));
    }
    if tempo_varies
        && let Some(s1) = fit::classify(&tracked)
        && !matches!(s1, Shape::Constant(_))
    {
        let obs = reseed(&finder, med, &|n| s1.secs_at(n), 0.035, duration);
        if let Some(shape) = fit::classify(&obs)
            && !matches!(shape, Shape::Constant(_))
        {
            candidates.push((shape, obs));
        }
    }
    timer.lap("candidates");

    // Produced music has a constant tempo. A flexible grid replaces the
    // constant one only when the constant grid clearly misses the kicks and
    // the flexible one clearly hits them; otherwise it would just follow
    // detection noise, and a wobbly grid on the master makes sync wobble.
    // One fixed set of real kick times (lattice and tracker observations),
    // so a grid that is far off in some section counts those kicks as misses.
    let mut kicks: Vec<f64> =
        lattice_obs.iter().chain(tracked.iter()).filter(|o| o.weight >= fit::STRONG).map(|o| o.secs).collect();
    kicks.sort_by(f64::total_cmp);
    kicks.dedup_by(|b, a| (*b - *a).abs() < 0.02);
    let hits: Vec<f64> = candidates.iter().map(|(sh, _)| hit_share(&kicks, sh)).collect();
    if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
        for ((sh, _), h) in candidates.iter().zip(&hits) {
            let kind = match sh {
                Shape::Constant(_) => "constant",
                Shape::Piecewise(_) => "piecewise",
                Shape::Live { .. } => "live",
            };
            eprintln!("candidate {kind}: kicks on grid {:.1} %", h * 100.0);
        }
    }
    let mut pick = 0;
    if hits[0] < 0.7 {
        for i in 1..candidates.len() {
            let simpler_first = matches!(candidates[i].0, Shape::Piecewise(_)) && hits[i] >= hits[pick] - 0.02;
            if hits[i] >= hits[0] + 0.25 && (hits[i] > hits[pick] || simpler_first) {
                pick = i;
            }
        }
    }
    let (mut shape, obs) = candidates.swap_remove(pick);
    let mut aligned_obs: Vec<Obs> = Vec::new();
    report.strong_beats = obs.iter().filter(|o| o.weight >= fit::STRONG).count();
    let mut flags = GridFlags::empty();
    if report.strong_beats < 16 {
        flags |= GridFlags::NO_RHYTHM;
    }
    // The other band's finder: template alignment and the phase fix look at both.
    let other = TransientFinder::new(mono, sr, if finder.band() == Band::Low { Band::Broad } else { Band::Low });
    let (low_f, broad_f) = if finder.band() == Band::Low { (&finder, &other) } else { (&other, &finder) };

    // A constant grid is refitted on template-aligned beats: each beat
    // matched against the track's average beat is timed far more steadily
    // than by its own attack, so the tempo comes out exact.
    if let Shape::Constant(f) = &shape {
        let mut current = f.clone();
        // Each pass sharpens the template, so a grid that starts off by
        // tens of ms at the ends converges in a few passes.
        for _ in 0..8 {
            let (lo, hi) = span(&|n| current.model.eval(n as f64), duration);
            let lines: Vec<(i64, f64)> = (lo..=hi)
                .map(|n| (n, current.model.eval(n as f64)))
                .filter(|(_, t)| *t > 0.1 && *t < duration - 0.1)
                .collect();
            let aligned = crate::phase::align_beats(low_f, broad_f, &lines);
            if aligned.len() < 32 {
                break;
            }
            let Some(refit) = fit::robust_fit(&aligned, &[], fit::HUBER_SECS, fit::REJECT_SECS) else { break };
            if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
                eprintln!(
                    "aligned: {} beats, bpm {:.4}, rms {:.2} ms",
                    aligned.len(),
                    60.0 / refit.model.coef[1],
                    refit.rms_ms()
                );
            }
            let (lo_i, hi_i) = (aligned[0].idx as f64, aligned[aligned.len() - 1].idx as f64);
            let moved = (refit.model.eval(lo_i) - current.model.eval(lo_i))
                .abs()
                .max((refit.model.eval(hi_i) - current.model.eval(hi_i)).abs());
            current = refit;
            kicks = aligned.iter().filter(|o| o.weight >= fit::STRONG).map(|o| o.secs).collect();
            aligned_obs = aligned;
            if moved < 0.0003 {
                break;
            }
        }
        shape = Shape::Constant(current);
    }
    let fit_obs = if aligned_obs.is_empty() { &obs } else { &aligned_obs };

    let mut snapped_bpm = None;
    if let Shape::Constant(f) = &shape {
        // Produced music runs at a round tempo. When noisy attacks leave the
        // fit a hair off (144.002), a round tempo wins if the kicks sit on
        // its lines at least as tightly.
        let snap = fit::snap_constant(fit_obs, f).or_else(|| {
            let base = tight_hits(&kicks, f);
            fit::round_tempos(fit_obs, f, LOOSE_SNAP_SECS, &[1.0, 0.5])
                .into_iter()
                .find(|(_, c)| tight_hits(&kicks, c) >= base - 0.01)
        });
        if let Some((bpm, snapped)) = snap {
            snapped_bpm = Some(bpm);
            shape = Shape::Constant(snapped);
            flags |= GridFlags::SNAPPED;
        }
    }
    // Beat-to-grid consistency from the aligned beats (before the phase fix,
    // which moves every line by the same amount).
    let aligned_stats = match &shape {
        Shape::Constant(f) if !aligned_obs.is_empty() => {
            let mut offs: Vec<f64> = aligned_obs
                .iter()
                .filter(|o| o.weight >= fit::STRONG)
                .map(|o| (o.secs - f.model.eval(o.idx as f64)).abs() * 1000.0)
                .collect();
            offs.sort_by(f64::total_cmp);
            (!offs.is_empty()).then(|| {
                let hit = offs.iter().filter(|o| **o <= 2.0).count() as f64 / offs.len() as f64;
                (hit, offs[offs.len() / 2], offs[(offs.len() * 95 / 100).min(offs.len() - 1)])
            })
        }
        _ => None,
    };
    timer.lap("classify+snap");

    // Exact phase: the start of the typical beat event.
    let (lo, hi) = span(&|n| shape.secs_at(n), duration);
    let lines: Vec<f64> = (lo..=hi).map(|n| shape.secs_at(n)).filter(|t| *t > 0.07 && *t < duration - 0.07).collect();
    // Live maps already sit on each beat's own attack; averaging beats that
    // wander would only blur them.
    let steady = !matches!(shape, Shape::Live { .. });
    if steady && let Some(fix) = crate::phase::beat_onset(low_f, broad_f, &lines) {
        if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
            eprintln!("phase fix: {:+.2} ms from {}", fix.shift * 1000.0, fix.source);
        }
        shape = shape.shifted(fix.shift);
        report.phase_source = fix.source;
    }
    // The refinements above follow the attacks within tens of ms, and can
    // walk onto a sound next to the kick (a hat or a bass note just before
    // it). If the finished constant grid is clearly off the kick, put it back.
    if let Shape::Constant(f) = &shape
        && let Some(k) = kick_fold.find(60.0 / f.model.coef[1], f.model.coef[0])
        && k.anchor_misses(false)
    {
        if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
            eprintln!("kick check: grid moved {:+.2} ms onto the kick", k.offset * 1000.0);
        }
        shape = shape.shifted(k.offset);
        report.phase_source = "kick fold";
    }
    drop(kick_fold);
    timer.lap("phase");
    let fit_ref = shape.fit();
    report.rms_ms = fit_ref.rms_ms();
    report.drift_ms = fit_ref.max_drift_ms(&obs, 16);
    report.bpm_fit = 60.0 / fit_ref.model.coef[1];

    // Bar phase from features over every beat index.
    let (first, last) = (obs[0].idx, obs[obs.len() - 1].idx);
    let times: Vec<f64> = (first..=last + 1).map(|n| shape.secs_at(n)).collect();
    let feats = downbeat::features(&dec.bands, dec.sr, &dec.chroma, &times);
    let first_strong = obs.iter().find(|o| o.weight >= fit::STRONG).map_or(first, |o| o.idx);
    drop(other);
    let db = downbeat::detect(&feats, first, first_strong);
    if std::env::var_os("RILLE_DEBUG_BARS").is_some() {
        // One line per bar of the chosen phase: band energies per beat.
        let start = (db.phase - first).rem_euclid(4) as usize;
        let len = feats.energy.len();
        for (b, i) in (start..len).step_by(4).enumerate() {
            let cells: Vec<String> = (i..(i + 4).min(len))
                .map(|j| format!("{:5.1} {:5.1} {:5.1}", feats.energy[j][0], feats.energy[j][1], feats.energy[j][2]))
                .collect();
            eprintln!("bar {b:4} beat {:5}: {}", first + i as i64, cells.join(" | "));
        }
    }
    if std::env::var_os("RILLE_DEBUG_OBS").is_some() {
        eprintln!("downbeat: {db:?}");
    }
    if std::env::var_os("RILLE_DOWNBEAT_DUMP").is_some() {
        let list = |v: &[f64]| v.iter().map(|x| format!("{x:.4}")).collect::<Vec<_>>().join(",");
        let cues: Vec<String> = feats.cues.iter().map(|c| format!("[{}]", list(c))).collect();
        let (nv, ev) = db.dump.clone().unwrap_or_default();
        let ev: Vec<String> = ev.iter().map(i64::to_string).collect();
        report.downbeat_dump = format!(
            r#"{{"first":{first},"phase":{},"trust":{:.4},"agreement":{:.4},"events":{},"phrase":[{}],"novelty":[{}],"model":[{}],"strong":{},"nv":[{}],"ev":[{}],"cues":[{}]}}"#,
            db.phase,
            db.trust,
            db.agreement,
            db.events,
            list(&db.phrase_share),
            list(&db.novelty_share),
            list(&db.model),
            first_strong.rem_euclid(4),
            list(&nv),
            ev.join(","),
            cues.join(",")
        );
    }
    let (phase, db_margin) = (db.phase, db.margin);
    report.downbeat_margin = db_margin;
    report.downbeat_agreement = db.agreement;
    report.downbeat_source = db.source;
    report.downbeat_events = db.events;

    // Beat 0 = first downbeat at or after the first reliable beat.
    let n0 = first_strong + (phase - first_strong).rem_euclid(4);

    let map = match &shape {
        Shape::Constant(f) => {
            let bpm = snapped_bpm.unwrap_or(60.0 / f.model.coef[1]);
            report.kind = "constant";
            BeatMap::Constant(ConstantMap::new(f.model.eval(n0 as f64), bpm).expect("finite fit"))
        }
        Shape::Piecewise(f) => {
            report.kind = "piecewise";
            flags |= GridFlags::TEMPO_CHANGE;
            let mut markers =
                vec![TempoMarker { secs: f.model.eval(n0 as f64), bpm: 60.0 / f.model.slope_at(n0 as f64) }];
            for &k in f.model.knots.iter().filter(|&&k| k > n0 as f64) {
                markers.push(TempoMarker { secs: f.model.eval(k), bpm: 60.0 / f.model.slope_at(k) });
            }
            BeatMap::Piecewise(PiecewiseMap::new(0.0, markers).expect("increasing markers"))
        }
        Shape::Live { first_idx, secs, .. } => {
            report.kind = "live";
            flags |= GridFlags::VARIABLE_TEMPO;
            let skip = (n0 - first_idx).max(0) as usize;
            BeatMap::Live(LiveMap::new(secs[skip.min(secs.len() - 2)..].to_vec()).expect("increasing beats"))
        }
    };
    let mut grid = BeatGrid::new(map, GridSource::Auto { analyzer_version: ANALYZER_VERSION });
    report.bpm = grid.bpm_at(grid.secs_at(0.0));

    // How well does the final grid sit on the attacks? Hit rate: share of
    // beats with a strong attack in the surrounding ±20 ms that also have one
    // within 5 ms of the line. Offsets: to the strongest attack within ±20 ms.
    let mut offsets = Vec::new();
    let (mut beats_with_attack, mut hits) = (0usize, 0usize);
    for n in (first - n0)..=(last - n0) {
        let g = grid.secs_at(n as f64);
        if let Some(tr) = finder.attack(g, 0.03)
            && norm(tr.strength, med) >= fit::STRONG
        {
            beats_with_attack += 1;
            hits += usize::from(attack_near(&finder, med, g).is_some());
            offsets.push((tr.secs - g).abs() * 1000.0);
        }
    }
    if let Some((hit, p50, p95)) = aligned_stats {
        report.hit_rate = hit;
        report.hit_p50_ms = p50;
        report.hit_p95_ms = p95;
    } else if beats_with_attack > 0 {
        report.hit_rate = hits as f64 / beats_with_attack as f64;
        offsets.sort_by(f64::total_cmp);
        report.hit_p50_ms = offsets[offsets.len() / 2];
        report.hit_p95_ms = offsets[(offsets.len() * 95 / 100).min(offsets.len() - 1)];
    }

    timer.lap("downbeat+report");
    // Warnings only where a DJ would really have to check: half or double
    // tempo nearly as likely, or section changes that disagree on bar 1.
    if tempo.octave_margin < 1.05 {
        flags |= GridFlags::OCTAVE_AMBIGUOUS;
    }
    let sections = db.trust >= 0.5;
    let db_sure = if sections { db.agreement } else { 0.7 * (db_margin / 0.2).min(1.0) };
    if (sections && db.agreement < 0.6) || (!sections && db_margin < 0.1) {
        flags |= GridFlags::DOWNBEAT_UNCERTAIN;
    }
    let om = ((tempo.octave_margin - 1.0) / 0.2).clamp(0.0, 1.0);
    let mut confidence = report.hit_rate.sqrt() * (0.8 + 0.2 * om) * (0.7 + 0.3 * db_sure);
    if flags.contains(GridFlags::NO_RHYTHM) {
        confidence *= 0.3;
    }
    grid.confidence = confidence as f32;
    if confidence < 0.6 {
        flags |= GridFlags::LOW_CONFIDENCE;
    }
    grid.flags = flags;
    (Some(grid), report)
}

/// Constant grid fitted to the attacks found along the lattice with one
/// detector band.
struct BandFit {
    finder: TransientFinder,
    /// Median attack strength, the unit for weights.
    med: f64,
    obs: Vec<Obs>,
    fit: fit::Fit,
    /// Share of all beats with a strong attack within 5 ms of the grid.
    consistency: f64,
    /// Share of lattice beats that have a clear attack in this band.
    attack_share: f64,
}

fn band_fit(
    finder: TransientFinder,
    lattice: &dyn Fn(i64) -> f64,
    duration: f64,
    allow_half_shift: bool,
) -> Option<BandFit> {
    let (lo, hi) = span(lattice, duration);
    let mut strengths: Vec<f64> =
        (lo..=hi).filter_map(|n| finder.find(lattice(n), 0.045)).map(|t| t.strength).collect();
    let med = median(&mut strengths).max(1e-6);
    let with_attack = strengths.iter().filter(|s| **s >= 0.5 * med && **s > 0.5).count();
    let obs = reseed(&finder, med, lattice, 0.045, duration);
    let first = fit::robust_fit(&obs, &[], fit::HUBER_SECS, fit::REJECT_SECS)?;
    // If attacks half a beat later are clearly stronger, the lattice sits on
    // the off-beat (e.g. a bassline); move to the beat.
    let line = |n: i64| first.model.eval(n as f64);
    let half = first.model.coef[1] / 2.0;
    let sum_at = |off: f64| -> f64 {
        let (lo, hi) = span(&line, duration);
        (lo..=hi).filter_map(|n| finder.find(line(n) + off, 0.02)).map(|t| norm(t.strength, med)).sum()
    };
    let shift = if allow_half_shift && sum_at(half) > sum_at(0.0) * 1.25 { half } else { 0.0 };
    let obs = reseed(&finder, med, &|n| line(n) + shift, 0.02, duration);
    let fit = fit::robust_fit(&obs, &[], fit::HUBER_SECS, fit::REJECT_SECS)?;
    let total = obs.len().max(1) as f64;
    let on_grid =
        obs.iter().zip(&fit.residuals).filter(|(o, r)| o.weight >= fit::STRONG && r.abs() < 0.005).count() as f64;
    let beats = (hi - lo + 1).max(1) as f64;
    Some(BandFit { finder, med, obs, fit, consistency: on_grid / total, attack_share: with_attack as f64 / beats })
}

fn norm(strength: f64, med: f64) -> f64 {
    if strength < 0.3 * med { 0.0 } else { (strength / med).min(2.0) }
}

/// Beat indices whose predicted times lie inside the track.
fn span(pred: &dyn Fn(i64) -> f64, duration: f64) -> (i64, i64) {
    let (mut lo, mut hi) = (0i64, 0i64);
    while lo > -100_000 && pred(lo - 1) > 0.02 {
        lo -= 1;
    }
    while hi < 100_000 && pred(hi + 1) < duration - 0.02 {
        hi += 1;
    }
    (lo, hi)
}

/// Looks for the attack at every predicted beat of the track.
fn reseed(finder: &TransientFinder, med: f64, pred: &dyn Fn(i64) -> f64, radius: f64, duration: f64) -> Vec<Obs> {
    let (lo, hi) = span(pred, duration);
    (lo..=hi)
        .filter_map(|n| {
            finder.attack(pred(n), radius).map(|tr| Obs { idx: n, secs: tr.secs, weight: norm(tr.strength, med) })
        })
        .collect()
}

/// Weight of a strong attack within 5 ms of `g`, if there is one. Other
/// sounds a little further away (a layered clap 15 ms after the kick) don't
/// matter: the question is whether the grid line sits on an attack.
fn attack_near(finder: &TransientFinder, med: f64, g: f64) -> Option<f64> {
    finder
        .attack(g, 0.03)
        .filter(|tr| (tr.secs - g).abs() <= 0.005)
        .map(|tr| norm(tr.strength, med))
        .filter(|w| *w >= fit::STRONG)
}

/// Share of the kick times within 5 ms of one of the shape's beat lines.
fn hit_share(kicks: &[f64], shape: &Shape) -> f64 {
    if kicks.is_empty() {
        return 0.0;
    }
    let near = |t: f64| {
        // Beat index near `t`: search the neighbouring integer beats.
        let (mut lo, mut hi) = (-100_000i64, 100_000i64);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if shape.secs_at(mid) <= t { lo = mid } else { hi = mid }
        }
        (t - shape.secs_at(lo)).abs().min((shape.secs_at(hi) - t).abs()) <= 0.005
    };
    kicks.iter().filter(|&&t| near(t)).count() as f64 / kicks.len() as f64
}

/// How far a round tempo may move the grid end to end before it counts as a
/// different tempo rather than detection noise.
const LOOSE_SNAP_SECS: f64 = 0.015;

/// Share of `kicks` within ±2.5 ms of a constant grid's lines, after moving
/// the grid onto their median offset (the phase is fixed separately).
fn tight_hits(kicks: &[f64], fit: &fit::Fit) -> f64 {
    let (a, p) = (fit.model.coef[0], fit.model.coef[1]);
    let res: Vec<f64> = kicks.iter().map(|&t| t - (a + ((t - a) / p).round() * p)).collect();
    let mut near: Vec<f64> = res.iter().copied().filter(|r| r.abs() <= 0.01).collect();
    if near.is_empty() {
        return 0.0;
    }
    near.sort_by(f64::total_cmp);
    let centre = near[near.len() / 2];
    res.iter().filter(|r| (*r - centre).abs() <= 0.0025).count() as f64 / kicks.len() as f64
}

/// Prints observations and their residuals to stderr when `RILLE_DEBUG_OBS` is set.
fn debug_obs(label: &str, obs: &[Obs]) {
    if std::env::var_os("RILLE_DEBUG_OBS").is_none() {
        return;
    }
    if let Some(f) = fit::robust_fit(obs, &[], fit::HUBER_SECS, fit::REJECT_SECS) {
        eprintln!("{label}: {} obs, bpm {:.4}", obs.len(), 60.0 / f.model.coef[1]);
        for (o, r) in obs.iter().zip(&f.residuals) {
            eprintln!("obs {:5} {:9.4}s w {:.2} res {:8.2} ms", o.idx, o.secs, o.weight, r * 1000.0);
        }
    }
}

/// Prints stage durations to stderr when `RILLE_TIMING` is set.
struct Timer(Option<std::cell::Cell<std::time::Instant>>);

impl Timer {
    fn new() -> Self {
        Self(std::env::var_os("RILLE_TIMING").map(|_| std::cell::Cell::new(std::time::Instant::now())))
    }

    fn lap(&self, what: &str) {
        if let Some(t) = &self.0 {
            eprintln!("  {what:16} {:7.1} ms", t.get().elapsed().as_secs_f64() * 1000.0);
            t.set(std::time::Instant::now());
        }
    }
}

/// Numbers beats. Strong beats are chained using the local period (a fit over
/// the previous 16 strong beats), which stays accurate across long gaps such
/// as breakdowns; weak beats then take the index of the nearest line position
/// or are dropped.
fn assign_indices(beats: &[(f64, f64)], beat_len: f64) -> Vec<Obs> {
    let strong: Vec<(f64, f64)> = beats.iter().copied().filter(|b| b.1 >= fit::STRONG).collect();
    let mut kept: Vec<(f64, f64)> = Vec::with_capacity(strong.len());
    let mut idx: Vec<i64> = Vec::with_capacity(strong.len());
    for &(t, w) in &strong {
        let Some(&(prev, _)) = kept.last() else {
            kept.push((t, w));
            idx.push(0);
            continue;
        };
        let from = kept.len().saturating_sub(16);
        let period = local_period(&kept[from..], &idx[from..]).unwrap_or(beat_len);
        let x = (t - prev) / period;
        // Off-beat hits (e.g. the bassline in a breakdown) are not beats.
        if (x - x.round()).abs() > 0.25 || x < 0.75 {
            continue;
        }
        kept.push((t, w));
        idx.push(idx[idx.len() - 1] + x.round() as i64);
    }
    let mut obs: Vec<Obs> = kept.iter().zip(&idx).map(|(&(secs, w), &i)| Obs { idx: i, secs, weight: w }).collect();
    if obs.len() >= 2 {
        // Weak beats: index from the surrounding strong beats.
        for &(t, w) in beats.iter().filter(|b| b.1 < fit::STRONG && b.1 > 0.0) {
            let j = obs.partition_point(|o| o.secs < t).clamp(1, obs.len() - 1);
            let (a, b) = (&obs[j - 1], &obs[j]);
            let p = (b.secs - a.secs) / (b.idx - a.idx) as f64;
            let n = a.idx as f64 + (t - a.secs) / p;
            if (n - n.round()).abs() < 0.2 {
                obs.push(Obs { idx: n.round() as i64, secs: t, weight: w });
            }
        }
        obs.sort_by_key(|o| o.idx);
        obs.dedup_by_key(|o| o.idx);
    }
    obs
}

/// Least-squares seconds-per-beat through `(secs, _)` points with indices.
fn local_period(pts: &[(f64, f64)], idx: &[i64]) -> Option<f64> {
    if pts.len() < 4 {
        return None;
    }
    let n = pts.len() as f64;
    let mx = idx.iter().map(|&i| i as f64).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (p, &i) in pts.iter().zip(idx) {
        sxy += (i as f64 - mx) * (p.0 - my);
        sxx += (i as f64 - mx).powi(2);
    }
    (sxx > 0.0).then(|| sxy / sxx)
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}
