//! Beatgrid model shared by analysis, engine and UI.
//!
//! All positions are `f64` seconds measured from the first decoded sample
//! (after encoder delay / priming has been removed), so grids are independent
//! of sample rate and map directly to Traktor NML (milliseconds).
//!
//! A beat index is a continuous `f64`: integer values are beats, the
//! fractional part is the phase within the beat. Beat indices extend to
//! negative values before the first detected beat, so the grid always covers
//! the whole track.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maps between time and beat position. Implementations must be strictly
/// monotone, and `secs_at` must be the inverse of `beat_at`.
pub trait BeatClock {
    fn beat_at(&self, secs: f64) -> f64;
    fn secs_at(&self, beat: f64) -> f64;
    fn bpm_at(&self, secs: f64) -> f64;
}

#[derive(Clone, Debug, PartialEq)]
pub enum GridError {
    InvalidBpm(f64),
    NonFinite,
    NotIncreasing { index: usize },
    TooFewBeats(usize),
    Empty,
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBpm(bpm) => write!(f, "invalid bpm {bpm}"),
            Self::NonFinite => write!(f, "non-finite position"),
            Self::NotIncreasing { index } => write!(f, "positions not strictly increasing at {index}"),
            Self::TooFewBeats(n) => write!(f, "live map needs at least 2 beats, got {n}"),
            Self::Empty => write!(f, "piecewise map needs at least one tempo marker"),
        }
    }
}

impl std::error::Error for GridError {}

fn check_bpm(bpm: f64) -> Result<(), GridError> {
    if bpm.is_finite() && bpm > 0.0 { Ok(()) } else { Err(GridError::InvalidBpm(bpm)) }
}

fn check_finite(v: f64) -> Result<(), GridError> {
    if v.is_finite() { Ok(()) } else { Err(GridError::NonFinite) }
}

// ---------------------------------------------------------------------------
// Constant tempo
// ---------------------------------------------------------------------------

/// A single tempo for the whole track: beat 0 sits at `anchor_secs`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawConstant")]
pub struct ConstantMap {
    anchor_secs: f64,
    bpm: f64,
}

#[derive(Deserialize)]
struct RawConstant {
    anchor_secs: f64,
    bpm: f64,
}

impl TryFrom<RawConstant> for ConstantMap {
    type Error = GridError;
    fn try_from(r: RawConstant) -> Result<Self, GridError> {
        Self::new(r.anchor_secs, r.bpm)
    }
}

impl ConstantMap {
    pub fn new(anchor_secs: f64, bpm: f64) -> Result<Self, GridError> {
        check_finite(anchor_secs)?;
        check_bpm(bpm)?;
        Ok(Self { anchor_secs, bpm })
    }

    pub fn anchor_secs(&self) -> f64 {
        self.anchor_secs
    }

    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    pub fn beat_secs(&self) -> f64 {
        60.0 / self.bpm
    }
}

impl BeatClock for ConstantMap {
    fn beat_at(&self, secs: f64) -> f64 {
        (secs - self.anchor_secs) * self.bpm / 60.0
    }

    fn secs_at(&self, beat: f64) -> f64 {
        self.anchor_secs + beat * 60.0 / self.bpm
    }

    fn bpm_at(&self, _secs: f64) -> f64 {
        self.bpm
    }
}

// ---------------------------------------------------------------------------
// Piecewise constant tempo
// ---------------------------------------------------------------------------

/// Where a new tempo starts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TempoMarker {
    pub secs: f64,
    pub bpm: f64,
}

/// A derived segment: `start_beat` is computed so the beat index is
/// continuous across segment boundaries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start_beat: f64,
    pub start_secs: f64,
    pub bpm: f64,
}

/// Piecewise-constant tempo, for tracks with distinct tempo sections or edits.
/// The first segment extends backwards and the last one forwards indefinitely.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawPiecewise", into = "RawPiecewise")]
pub struct PiecewiseMap {
    first_beat: f64,
    markers: Vec<TempoMarker>,
    segs: Vec<Segment>,
}

#[derive(Serialize, Deserialize)]
struct RawPiecewise {
    first_beat: f64,
    markers: Vec<TempoMarker>,
}

impl TryFrom<RawPiecewise> for PiecewiseMap {
    type Error = GridError;
    fn try_from(r: RawPiecewise) -> Result<Self, GridError> {
        Self::new(r.first_beat, r.markers)
    }
}

impl From<PiecewiseMap> for RawPiecewise {
    fn from(m: PiecewiseMap) -> Self {
        Self { first_beat: m.first_beat, markers: m.markers }
    }
}

impl PiecewiseMap {
    /// `first_beat` is the beat index at the first marker's position.
    pub fn new(first_beat: f64, markers: Vec<TempoMarker>) -> Result<Self, GridError> {
        check_finite(first_beat)?;
        if markers.is_empty() {
            return Err(GridError::Empty);
        }
        let mut segs = Vec::with_capacity(markers.len());
        let mut beat = first_beat;
        for (i, m) in markers.iter().enumerate() {
            check_finite(m.secs)?;
            check_bpm(m.bpm)?;
            if let Some(prev) = segs.last() {
                let prev: &Segment = prev;
                if m.secs <= prev.start_secs {
                    return Err(GridError::NotIncreasing { index: i });
                }
                beat = prev.start_beat + (m.secs - prev.start_secs) * prev.bpm / 60.0;
            }
            segs.push(Segment { start_beat: beat, start_secs: m.secs, bpm: m.bpm });
        }
        Ok(Self { first_beat, markers, segs })
    }

    pub fn markers(&self) -> &[TempoMarker] {
        &self.markers
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segs
    }

    fn seg_for_secs(&self, secs: f64) -> &Segment {
        let i = self.segs.partition_point(|s| s.start_secs <= secs);
        &self.segs[i.saturating_sub(1)]
    }

    fn seg_for_beat(&self, beat: f64) -> &Segment {
        let i = self.segs.partition_point(|s| s.start_beat <= beat);
        &self.segs[i.saturating_sub(1)]
    }
}

impl BeatClock for PiecewiseMap {
    fn beat_at(&self, secs: f64) -> f64 {
        let s = self.seg_for_secs(secs);
        s.start_beat + (secs - s.start_secs) * s.bpm / 60.0
    }

    fn secs_at(&self, beat: f64) -> f64 {
        let s = self.seg_for_beat(beat);
        s.start_secs + (beat - s.start_beat) * 60.0 / s.bpm
    }

    fn bpm_at(&self, secs: f64) -> f64 {
        self.seg_for_secs(secs).bpm
    }
}

// ---------------------------------------------------------------------------
// Live (one marker per beat)
// ---------------------------------------------------------------------------

/// One position per beat for music without a steady tempo (live drums,
/// rubato). Beat `i` sits at `beats_secs[i]`; tempo is constant between two
/// adjacent beats, which keeps the map exactly invertible. Before the first and
/// after the last beat the edge interval's tempo is extended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawLive")]
pub struct LiveMap {
    beats_secs: Vec<f64>,
}

#[derive(Deserialize)]
struct RawLive {
    beats_secs: Vec<f64>,
}

impl TryFrom<RawLive> for LiveMap {
    type Error = GridError;
    fn try_from(r: RawLive) -> Result<Self, GridError> {
        Self::new(r.beats_secs)
    }
}

impl LiveMap {
    pub fn new(beats_secs: Vec<f64>) -> Result<Self, GridError> {
        if beats_secs.len() < 2 {
            return Err(GridError::TooFewBeats(beats_secs.len()));
        }
        for (i, w) in beats_secs.windows(2).enumerate() {
            check_finite(w[0])?;
            check_finite(w[1])?;
            if w[1] <= w[0] {
                return Err(GridError::NotIncreasing { index: i + 1 });
            }
        }
        Ok(Self { beats_secs })
    }

    pub fn beats_secs(&self) -> &[f64] {
        &self.beats_secs
    }

    /// Index `i` of the interval `[beats[i], beats[i + 1]]` used for `secs`,
    /// clamped to the first/last interval for extrapolation.
    fn interval_for_secs(&self, secs: f64) -> usize {
        let n = self.beats_secs.len();
        self.beats_secs.partition_point(|&t| t <= secs).saturating_sub(1).min(n - 2)
    }

    fn interval_for_beat(&self, beat: f64) -> usize {
        let last = (self.beats_secs.len() - 2) as f64;
        beat.floor().clamp(0.0, last) as usize
    }
}

impl BeatClock for LiveMap {
    fn beat_at(&self, secs: f64) -> f64 {
        let i = self.interval_for_secs(secs);
        let (t0, t1) = (self.beats_secs[i], self.beats_secs[i + 1]);
        i as f64 + (secs - t0) / (t1 - t0)
    }

    fn secs_at(&self, beat: f64) -> f64 {
        let i = self.interval_for_beat(beat);
        let (t0, t1) = (self.beats_secs[i], self.beats_secs[i + 1]);
        t0 + (beat - i as f64) * (t1 - t0)
    }

    fn bpm_at(&self, secs: f64) -> f64 {
        let i = self.interval_for_secs(secs);
        60.0 / (self.beats_secs[i + 1] - self.beats_secs[i])
    }
}

// ---------------------------------------------------------------------------
// BeatMap + BeatGrid
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BeatMap {
    Constant(ConstantMap),
    Piecewise(PiecewiseMap),
    Live(LiveMap),
}

impl BeatMap {
    pub fn constant(anchor_secs: f64, bpm: f64) -> Result<Self, GridError> {
        ConstantMap::new(anchor_secs, bpm).map(Self::Constant)
    }

    fn clock(&self) -> &dyn BeatClock {
        match self {
            Self::Constant(m) => m,
            Self::Piecewise(m) => m,
            Self::Live(m) => m,
        }
    }
}

impl BeatClock for BeatMap {
    fn beat_at(&self, secs: f64) -> f64 {
        self.clock().beat_at(secs)
    }

    fn secs_at(&self, beat: f64) -> f64 {
        self.clock().secs_at(beat)
    }

    fn bpm_at(&self, secs: f64) -> f64 {
        self.clock().bpm_at(secs)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GridSource {
    Auto { analyzer_version: u32 },
    Manual,
    ImportedNml,
    Tapped,
}

bitflags! {
    /// Reasons a grid may need attention; shown as badges in deck and library.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct GridFlags: u32 {
        const LOW_CONFIDENCE     = 1 << 0;
        const OCTAVE_AMBIGUOUS   = 1 << 1;
        const DOWNBEAT_UNCERTAIN = 1 << 2;
        const VARIABLE_TEMPO     = 1 << 3;
        const TEMPO_CHANGE       = 1 << 4;
        const BAR_PHASE_SHIFT    = 1 << 5;
        const NO_RHYTHM          = 1 << 6;
        const SNAPPED            = 1 << 7;
        const ENGINES_DISAGREE   = 1 << 8;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeatGrid {
    pub map: BeatMap,
    pub beats_per_bar: u8,
    /// A beat index that falls on a downbeat; bar phase is measured from it.
    pub downbeat_beat_index: i64,
    pub source: GridSource,
    /// Locked grids are never replaced by re-analysis or AutoGrid.
    pub locked: bool,
    /// 0..1, see the analysis confidence model.
    pub confidence: f32,
    pub flags: GridFlags,
}

impl BeatGrid {
    pub fn new(map: BeatMap, source: GridSource) -> Self {
        Self {
            map,
            beats_per_bar: 4,
            downbeat_beat_index: 0,
            source,
            locked: false,
            confidence: 1.0,
            flags: GridFlags::empty(),
        }
    }

    /// Position within the bar in beats, in `[0, beats_per_bar)`.
    pub fn bar_phase(&self, beat: f64) -> f64 {
        (beat - self.downbeat_beat_index as f64).rem_euclid(f64::from(self.beats_per_bar.max(1)))
    }

    pub fn is_downbeat(&self, beat_index: i64) -> bool {
        (beat_index - self.downbeat_beat_index).rem_euclid(i64::from(self.beats_per_bar.max(1))) == 0
    }
}

impl BeatClock for BeatGrid {
    fn beat_at(&self, secs: f64) -> f64 {
        self.map.beat_at(secs)
    }

    fn secs_at(&self, beat: f64) -> f64 {
        self.map.secs_at(beat)
    }

    fn bpm_at(&self, secs: f64) -> f64 {
        self.map.bpm_at(secs)
    }
}

// ---------------------------------------------------------------------------
// Editing (user corrections)
// ---------------------------------------------------------------------------

impl BeatMap {
    /// All beats moved by `secs`.
    pub fn shifted(&self, secs: f64) -> Self {
        match self {
            Self::Constant(m) => Self::Constant(ConstantMap { anchor_secs: m.anchor_secs + secs, bpm: m.bpm }),
            Self::Piecewise(m) => {
                let markers = m.markers.iter().map(|t| TempoMarker { secs: t.secs + secs, bpm: t.bpm }).collect();
                Self::Piecewise(PiecewiseMap::new(m.first_beat, markers).expect("shift keeps markers valid"))
            }
            Self::Live(m) => Self::Live(LiveMap { beats_secs: m.beats_secs.iter().map(|t| t + secs).collect() }),
        }
    }

    /// Tempo multiplied by `factor` (e.g. 2 or ½), keeping the beat nearest
    /// to `pivot_secs` in place.
    pub fn scaled(&self, factor: f64, pivot_secs: f64) -> Self {
        let pivot = self.secs_at(self.beat_at(pivot_secs).round());
        match self {
            Self::Constant(m) => {
                let bpm = m.bpm * factor;
                Self::Constant(ConstantMap { anchor_secs: pivot, bpm })
            }
            Self::Piecewise(m) => {
                let markers: Vec<TempoMarker> =
                    m.markers.iter().map(|t| TempoMarker { secs: t.secs, bpm: t.bpm * factor }).collect();
                let first = m.first_beat * factor;
                let scaled = PiecewiseMap::new(first, markers).expect("scaling keeps markers valid");
                // Re-anchor on the pivot beat.
                let drift = pivot - scaled.secs_at(scaled.beat_at(pivot).round());
                Self::Piecewise(scaled).shifted(drift)
            }
            Self::Live(m) => {
                let b = &m.beats_secs;
                let beats: Vec<f64> = if factor >= 2.0 {
                    // Insert a beat between each pair.
                    let mut v = Vec::with_capacity(b.len() * 2);
                    for w in b.windows(2) {
                        v.push(w[0]);
                        v.push(0.5 * (w[0] + w[1]));
                    }
                    v.push(b[b.len() - 1]);
                    v
                } else if factor <= 0.5 {
                    b.iter().step_by(2).copied().collect()
                } else {
                    b.clone()
                };
                match LiveMap::new(beats) {
                    Ok(l) => Self::Live(l),
                    Err(_) => self.clone(),
                }
            }
        }
    }
}

impl BeatGrid {
    /// Moves the grid so the beat nearest to `secs` lands exactly on it.
    pub fn with_beat_at(&self, secs: f64) -> Self {
        let nearest = self.secs_at(self.beat_at(secs).round());
        self.edited(self.map.shifted(secs - nearest))
    }

    /// Marks the beat nearest to `secs` as a downbeat.
    pub fn with_downbeat_at(&self, secs: f64) -> Self {
        let mut g = self.edited(self.map.clone());
        g.downbeat_beat_index = self.beat_at(secs).round() as i64;
        g
    }

    pub fn shifted(&self, secs: f64) -> Self {
        self.edited(self.map.shifted(secs))
    }

    /// Double (`2.0`) or halve (`0.5`) the tempo around `pivot_secs`.
    pub fn scaled(&self, factor: f64, pivot_secs: f64) -> Self {
        let mut g = self.edited(self.map.scaled(factor, pivot_secs));
        let db = self.secs_at(self.downbeat_beat_index as f64);
        g.downbeat_beat_index = g.beat_at(db).round() as i64;
        g
    }

    /// A constant grid from tapped beat times (least squares through them).
    pub fn from_taps(taps: &[f64]) -> Option<Self> {
        if taps.len() < 4 {
            return None;
        }
        let n = taps.len() as f64;
        let mx = (n - 1.0) / 2.0;
        let my = taps.iter().sum::<f64>() / n;
        let (mut sxy, mut sxx) = (0.0, 0.0);
        for (i, t) in taps.iter().enumerate() {
            sxy += (i as f64 - mx) * (t - my);
            sxx += (i as f64 - mx).powi(2);
        }
        let period = sxy / sxx;
        let bpm = 60.0 / period;
        let anchor = my - mx * period;
        let map = BeatMap::constant(anchor, bpm).ok()?;
        Some(Self::new(map, GridSource::Tapped))
    }

    fn edited(&self, map: BeatMap) -> Self {
        Self { map, source: GridSource::Manual, confidence: 1.0, flags: GridFlags::empty(), ..self.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn constant_basics() {
        let m = ConstantMap::new(0.25, 120.0).unwrap();
        assert_abs_diff_eq!(m.beat_at(0.25), 0.0);
        assert_abs_diff_eq!(m.beat_at(0.75), 1.0);
        assert_abs_diff_eq!(m.beat_at(0.0), -0.5);
        assert_abs_diff_eq!(m.secs_at(8.0), 4.25);
    }

    #[test]
    fn rejects_invalid() {
        assert!(ConstantMap::new(0.0, 0.0).is_err());
        assert!(ConstantMap::new(f64::NAN, 120.0).is_err());
        assert!(LiveMap::new(vec![1.0]).is_err());
        assert!(LiveMap::new(vec![1.0, 1.0]).is_err());
        assert!(PiecewiseMap::new(0.0, vec![]).is_err());
        let dup = vec![TempoMarker { secs: 1.0, bpm: 120.0 }, TempoMarker { secs: 1.0, bpm: 130.0 }];
        assert!(PiecewiseMap::new(0.0, dup).is_err());
    }

    #[test]
    fn piecewise_is_continuous_at_boundaries() {
        let m =
            PiecewiseMap::new(0.0, vec![TempoMarker { secs: 0.0, bpm: 120.0 }, TempoMarker { secs: 10.0, bpm: 90.0 }])
                .unwrap();
        // 10 s at 120 bpm = 20 beats
        assert_abs_diff_eq!(m.beat_at(10.0), 20.0, epsilon = 1e-12);
        assert_abs_diff_eq!(m.beat_at(10.0 - 1e-9), 20.0, epsilon = 1e-6);
        assert_abs_diff_eq!(m.beat_at(12.0), 23.0, epsilon = 1e-12);
        assert_abs_diff_eq!(m.bpm_at(9.0), 120.0);
        assert_abs_diff_eq!(m.bpm_at(11.0), 90.0);
        assert_abs_diff_eq!(m.secs_at(23.0), 12.0, epsilon = 1e-12);
    }

    #[test]
    fn live_interpolates_and_extrapolates() {
        let m = LiveMap::new(vec![1.0, 1.5, 2.1, 2.6]).unwrap();
        assert_abs_diff_eq!(m.beat_at(1.5), 1.0, epsilon = 1e-12);
        assert_abs_diff_eq!(m.beat_at(1.8), 1.5, epsilon = 1e-12);
        assert_abs_diff_eq!(m.beat_at(0.5), -1.0, epsilon = 1e-12);
        assert_abs_diff_eq!(m.beat_at(3.1), 4.0, epsilon = 1e-12);
        assert_abs_diff_eq!(m.bpm_at(1.8), 100.0, epsilon = 1e-9);
    }

    #[test]
    fn bar_phase_and_downbeats() {
        let mut g = BeatGrid::new(BeatMap::constant(0.0, 120.0).unwrap(), GridSource::Manual);
        g.downbeat_beat_index = 1;
        assert_abs_diff_eq!(g.bar_phase(1.0), 0.0);
        assert_abs_diff_eq!(g.bar_phase(0.5), 3.5);
        assert!(g.is_downbeat(-3));
        assert!(g.is_downbeat(5));
        assert!(!g.is_downbeat(4));
    }

    #[test]
    fn edits() {
        let g = BeatGrid::new(BeatMap::constant(0.1, 120.0).unwrap(), GridSource::Manual);
        let moved = g.with_beat_at(0.61);
        assert_abs_diff_eq!(moved.secs_at(1.0), 0.61, epsilon = 1e-12);
        let doubled = g.scaled(2.0, 10.1);
        assert_abs_diff_eq!(doubled.bpm_at(0.0), 240.0);
        assert_abs_diff_eq!(doubled.secs_at(doubled.beat_at(10.1).round()), 10.1, epsilon = 1e-9);
        let d = g.with_downbeat_at(0.6);
        assert!(d.is_downbeat(1));
        let taps: Vec<f64> = (0..8).map(|i| 1.0 + i as f64 * 0.48 + if i % 2 == 0 { 0.003 } else { -0.003 }).collect();
        let t = BeatGrid::from_taps(&taps).unwrap();
        assert!((t.bpm_at(0.0) - 125.0).abs() < 0.5);
        let live = BeatMap::Live(LiveMap::new(vec![0.0, 0.5, 1.0, 1.5]).unwrap());
        let halved = live.scaled(0.5, 0.0);
        assert_abs_diff_eq!(halved.secs_at(1.0), 1.0);
    }

    #[test]
    fn serde_roundtrip_and_validation() {
        let g = BeatGrid::new(
            BeatMap::Piecewise(
                PiecewiseMap::new(
                    -2.0,
                    vec![TempoMarker { secs: 0.1, bpm: 128.0 }, TempoMarker { secs: 60.0, bpm: 130.0 }],
                )
                .unwrap(),
            ),
            GridSource::Auto { analyzer_version: 1 },
        );
        let json = serde_json::to_string(&g).unwrap();
        let back: BeatGrid = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);

        let bad = r#"{"kind":"constant","anchor_secs":0.0,"bpm":-1.0}"#;
        assert!(serde_json::from_str::<BeatMap>(bad).is_err());
    }
}
