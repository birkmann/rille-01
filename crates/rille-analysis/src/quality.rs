//! How well a grid sits on the kicks — the check behind "lines on the kicks".

use rille_core::{BeatClock, BeatGrid};

use crate::refine::TransientFinder;

#[derive(Clone, Debug, Default)]
pub struct KickAlignment {
    /// Beats with a clear kick attack near the grid line.
    pub kicks: usize,
    /// All grid beats inside the track.
    pub beats: usize,
    /// Median signed distance kick start − grid line (ms); positive = kick later.
    pub offset_ms: f64,
    /// Share of kicks whose start is within ±2 ms of the line.
    pub hit_2ms: f64,
    /// 95th percentile of |kick start − line| (ms).
    pub p95_ms: f64,
}

/// Measures every grid beat against the kick attack found within ±30 ms.
/// `low` must be a kick-band finder. Only clear attacks count (strength at
/// least half the median), so beats without a kick are ignored.
pub fn kick_alignment(low: &TransientFinder, grid: &BeatGrid, duration: f64) -> KickAlignment {
    let (b0, b1) = (grid.beat_at(0.05).ceil() as i64, grid.beat_at(duration - 0.05).floor() as i64);
    let found: Vec<(f64, f64)> = (b0..=b1)
        .filter_map(|b| {
            let g = grid.secs_at(b as f64);
            low.attack(g, 0.03).map(|t| ((t.secs - g) * 1000.0, t.strength))
        })
        .collect();
    let mut strengths: Vec<f64> = found.iter().map(|f| f.1).collect();
    strengths.sort_by(f64::total_cmp);
    let med = strengths.get(strengths.len() / 2).copied().unwrap_or(0.0);
    let mut offs: Vec<f64> = found.iter().filter(|f| f.1 >= 0.5 * med && f.1 > 0.5).map(|f| f.0).collect();
    let mut out = KickAlignment { kicks: offs.len(), beats: (b1 - b0 + 1).max(0) as usize, ..Default::default() };
    if offs.is_empty() {
        return out;
    }
    out.hit_2ms = offs.iter().filter(|o| o.abs() <= 2.0).count() as f64 / offs.len() as f64;
    offs.sort_by(f64::total_cmp);
    out.offset_ms = offs[offs.len() / 2];
    let mut abs: Vec<f64> = offs.iter().map(|o| o.abs()).collect();
    abs.sort_by(f64::total_cmp);
    out.p95_ms = abs[(abs.len() * 95 / 100).min(abs.len() - 1)];
    out
}
