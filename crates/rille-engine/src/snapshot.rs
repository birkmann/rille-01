//! State published by the audio thread after every block.

use rille_core::remix::{CELLS, SLOTS};

use crate::types::{FX_UNITS, HOTCUES, Hotcue, MAX_DECKS};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeckState {
    /// A track, or a remix deck.
    pub loaded: bool,
    /// This is a remix deck (see [`Snapshot::remix`]).
    pub remix: bool,
    pub track_id: u64,
    pub duration_secs: f64,
    /// Position you hear now (latency of time-stretching removed), seconds.
    pub position_secs: f64,
    pub playing: bool,
    pub sync: bool,
    pub master: bool,
    pub keylock: bool,
    pub flux: bool,
    pub reverse: bool,
    pub tick: bool,
    /// Times sync jumped this deck into phase since it was loaded.
    pub realigns: u32,
    /// Current playback speed relative to the original (1.0 = original).
    pub rate: f64,
    /// Tempo fader value 0..1.
    pub tempo_fader: f32,
    /// Track tempo at the play position, before `rate`.
    pub track_bpm: f64,
    pub bpm: f64,
    pub key_shift: f32,
    pub main_cue_secs: f64,
    pub hotcues: [Option<Hotcue>; HOTCUES],
    pub loop_start_secs: f64,
    pub loop_end_secs: f64,
    pub loop_set: bool,
    pub loop_active: bool,
    pub loop_size_idx: usize,
    /// Beat position at the play position (grid beat index, fractional).
    pub beat: f64,
    /// Phase difference to the master clock in beats (−0.5..0.5).
    pub phase_error: f64,
    /// Seconds per output frame the position advanced in the last block.
    pub speed: f64,
    pub cue_held: bool,
    pub end_warning: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelState {
    pub gain: f32,
    pub eq: [f32; 3],
    /// EQ kills held (low, mid, high).
    pub kill: [bool; 3],
    pub filter: f32,
    pub volume: f32,
    pub pfl: bool,
    pub fx_assign: [bool; FX_UNITS],
    /// Post-fader peak, linear, per side.
    pub meter: [f32; 2],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FxState {
    pub on: bool,
    pub dry_wet: f32,
    pub effects: [usize; 3],
    /// Per slot: the effect's parameters and buttons.
    pub knobs: [[f32; 3]; 3],
    pub buttons: [[bool; 3]; 3],
    /// Per slot: amount and on/off (group mode).
    pub amount: [f32; 3],
    pub enabled: [bool; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemixCellState {
    pub loaded: bool,
    pub looped: bool,
    /// Index into `rille_core::remix::COLORS`.
    pub color: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemixSlotState {
    /// Cell playing (`0..64`).
    pub cell: Option<u8>,
    /// Cell waiting for its quantize boundary.
    pub queued: Option<u8>,
    /// Position in the playing cell's sample, `0..1`.
    pub progress: f32,
    pub volume: f32,
    pub filter: f32,
    pub muted: bool,
    /// Post-fader peak, linear.
    pub meter: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemixState {
    /// The deck is a remix deck; everything else is unset otherwise.
    pub active: bool,
    /// Page of the pad grid, `0..4`.
    pub page: u8,
    pub quantize: bool,
    /// Index into `REMIX_QUANT_SIZES`.
    pub quant_idx: u8,
    /// Deck CAPTURE takes loops from.
    pub capture_source: u8,
    pub cells: [RemixCellState; CELLS],
    pub slots: [RemixSlotState; SLOTS],
}

impl Default for RemixState {
    fn default() -> Self {
        Self {
            active: false,
            page: 0,
            quantize: false,
            quant_idx: 0,
            capture_source: 0,
            cells: [RemixCellState::default(); CELLS],
            slots: [RemixSlotState::default(); SLOTS],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub decks: [DeckState; MAX_DECKS],
    pub channels: [ChannelState; MAX_DECKS],
    pub remix: [RemixState; MAX_DECKS],
    pub fx: [FxState; FX_UNITS],
    pub crossfader: f32,
    pub crossfader_curve: f32,
    pub crossfader_reverse: bool,
    pub main_level: f32,
    pub cue_mix: f32,
    pub cue_volume: f32,
    pub quantize: bool,
    pub snap: bool,
    pub limiter: bool,
    pub master_meter: [f32; 2],
    pub limiter_reduction_db: f32,
    /// Deck leading the tempo, if any.
    pub master_deck: Option<u8>,
    pub clock_bpm: f64,
    /// Master clock beat position (continuous).
    pub clock_beat: f64,
    pub sample_rate: u32,
    /// Output frames rendered since start; with `time_nanos` lets the UI
    /// extrapolate positions between snapshots.
    pub frames: u64,
    pub time_nanos: u64,
    /// Share of the block time spent rendering (0..1).
    pub cpu_load: f32,
}

impl Snapshot {
    /// Crossfader gain for `deck`'s channel (A/C left, B/D right), with the
    /// curve and reverse switch applied.
    pub fn crossfader_gain(&self, deck: usize) -> f32 {
        let x = if self.crossfader_reverse { 1.0 - self.crossfader } else { self.crossfader };
        crate::mixer::crossfader_gain(x, deck % 2, self.crossfader_curve)
    }
}
