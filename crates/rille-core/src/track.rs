//! Per-track data stored by the library: cue points and analysis results.

use crate::{BeatGrid, Key};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueKind {
    Cue,
    FadeIn,
    FadeOut,
    Load,
    Grid,
    Loop,
}

/// A stored cue or loop. `slot` 0..=7 is a hotcue button; `None` is a memory
/// cue that has no button.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CuePoint {
    pub slot: Option<u8>,
    pub kind: CueKind,
    pub start_secs: f64,
    /// Loop length; 0 for anything but loops.
    pub len_secs: f64,
    pub name: String,
    /// 0xRRGGBB
    pub color: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackCues {
    /// The deck's CUE point.
    pub main_cue_secs: f64,
    pub points: Vec<CuePoint>,
}

impl TrackCues {
    pub fn hotcue(&self, slot: u8) -> Option<&CuePoint> {
        self.points.iter().find(|p| p.slot == Some(slot))
    }

    /// Replaces whatever is in `cue.slot`.
    pub fn set_hotcue(&mut self, cue: CuePoint) {
        self.points.retain(|p| p.slot.is_none() || p.slot != cue.slot);
        self.points.push(cue);
    }

    pub fn delete_hotcue(&mut self, slot: u8) {
        self.points.retain(|p| p.slot != Some(slot));
    }
}

/// Default hotcue colors by slot.
pub const HOTCUE_COLORS: [u32; 8] = [0x2ec4b6, 0xf5a524, 0xe0565b, 0x7b8cff, 0x5fd068, 0xd96bd6, 0xf2c94c, 0x4fb3e8];

/// Bump when analysis results change so stale ones are recomputed.
pub const ANALYZER_VERSION: u32 = 3;

/// Output of track analysis, stored in the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackAnalysis {
    pub analyzer_version: u32,
    pub duration_secs: f64,
    pub sample_rate: u32,
    pub grid: Option<BeatGrid>,
    pub key: Option<Key>,
    pub key_confidence: f32,
    /// Integrated loudness (EBU R128 style), LUFS.
    pub lufs: Option<f32>,
    /// Sample peak, dBFS.
    pub peak_db: Option<f32>,
    /// First non-silent position.
    pub first_sound_secs: f64,
}
