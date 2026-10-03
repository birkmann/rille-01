//! Core types shared across the workspace. No audio, IO or UI dependencies.

pub mod beatgrid;
pub mod control;
pub mod drums;
pub mod ids;
pub mod key;
pub mod quantize;
pub mod remix;
pub mod track;
pub mod waveform;

pub use beatgrid::{BeatClock, BeatGrid, BeatMap, GridFlags, GridSource};
pub use control::{Control, ControlEvent, ControlKind, ControlTarget, ControlValue, Scope};
pub use ids::{DeckId, FxUnitId};
pub use key::Key;
pub use track::{CueKind, CuePoint, TrackAnalysis, TrackCues};
pub use waveform::WaveformSummary;
