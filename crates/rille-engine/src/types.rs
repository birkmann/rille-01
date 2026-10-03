//! Data passed between the app and the audio thread.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use rille_core::{BeatGrid, ControlEvent, CueKind};

use crate::drums::{DrumKit, DrumParams};
use crate::remix::{RemixDeck, RemixSample};

pub const MAX_DECKS: usize = rille_core::ids::MAX_DECKS;
pub const FX_UNITS: usize = 2;
pub const HOTCUES: usize = 8;

/// Loop / beatjump sizes selectable with `LoopSizeUp/Down`.
pub const LOOP_SIZES: [f64; 11] = [1.0 / 32.0, 1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
pub const DEFAULT_LOOP_SIZE: usize = 7;

/// Decoded track audio at its native sample rate.
pub struct TrackAudio {
    pub sample_rate: u32,
    pub frames: Vec<[f32; 2]>,
}

impl std::fmt::Debug for TrackAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TrackAudio({} frames at {} Hz)", self.frames.len(), self.sample_rate)
    }
}

impl TrackAudio {
    pub fn duration_secs(&self) -> f64 {
        self.frames.len() as f64 / f64::from(self.sample_rate.max(1))
    }
}

/// Stems of a track, in the order of the stem controls: drums, bass, other
/// (everything else), vocals.
pub const STEMS: usize = 4;
pub const STEM_NAMES: [&str; STEMS] = ["Drums", "Bass", "Other", "Vocals"];

/// Drums, bass and vocals of a track as 16-bit stereo, interleaved per frame
/// (drums L R, bass L R, vocals L R), at the track's sample rate and length.
/// The other stem is the track minus these three, so the stems add up to the
/// track exactly.
pub struct StemAudio {
    pub sample_rate: u32,
    pub frames: Vec<[i16; 6]>,
}

impl std::fmt::Debug for StemAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StemAudio({} frames at {} Hz)", self.frames.len(), self.sample_rate)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hotcue {
    pub secs: f64,
    pub kind: CueKind,
    /// Loop length (loop hotcues only).
    pub len_secs: f64,
}

/// Everything the engine needs to play a track.
pub struct LoadedTrack {
    /// App-side identity, echoed in snapshots and events.
    pub id: u64,
    pub audio: Arc<TrackAudio>,
    pub grid: Option<Arc<BeatGrid>>,
    pub main_cue_secs: f64,
    pub hotcues: [Option<Hotcue>; HOTCUES],
    /// Gain applied before the GAIN knob, e.g. from loudness analysis.
    pub auto_gain_db: f32,
}

/// Engine-wide options set from the app's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Tempo fader range, e.g. 0.08 for ±8 %.
    pub tempo_range: f64,
    /// Output channels 3/4 carry the headphone (cue) mix when available.
    pub cue_on_outputs_3_4: bool,
    /// On 2-channel interfaces: left = cue (mono), right = master (mono).
    pub split_cue: bool,
    pub limiter_ceiling_db: f32,
    /// Crossfade length for jumps and loop wraps.
    pub declick_ms: f32,
    /// External mixer mode: deck `i` plays on the stereo output pair
    /// `outputs[i]` (0 = outputs 1/2, 1 = 3/4, …; `None` = not routed), with
    /// its GAIN, EQ, filter and FX but no fader, crossfader, headphone cue or
    /// main mix: a hardware mixer does those. `None` = the internal mixer.
    pub external_outputs: Option<[Option<u8>; MAX_DECKS]>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tempo_range: 0.08,
            cue_on_outputs_3_4: true,
            split_cue: false,
            limiter_ceiling_db: -0.3,
            declick_ms: 3.0,
            external_outputs: None,
        }
    }
}

// `Load` is large on purpose: boxing it would free the box on the audio
// thread. Commands live in a preallocated ring buffer, so size is harmless.
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Buttons, knobs, faders and encoders (see `rille_core::Control`).
    Control(ControlEvent),
    Load {
        deck: u8,
        track: LoadedTrack,
    },
    Unload {
        deck: u8,
    },
    /// Replace the beatgrid (analysis finished or the user edited it).
    SetGrid {
        deck: u8,
        track_id: u64,
        grid: Option<Arc<BeatGrid>>,
    },
    SeekSecs {
        deck: u8,
        secs: f64,
    },
    Settings(Settings),
    /// Master clock tempo used while no deck leads.
    SetClockBpm(f64),
    /// Make `deck` the tempo master (`None` = internal clock).
    SetMaster(Option<u8>),
    /// Turn `deck` into a remix deck (`Some`, see [`RemixDeck::new`]) or back
    /// into an empty track deck.
    SetRemix {
        deck: u8,
        remix: Option<Box<RemixDeck>>,
    },
    /// Load (or empty) cell `cell` (`0..64`) of a remix deck.
    SetRemixCell {
        deck: u8,
        cell: u8,
        sample: Option<RemixSample>,
    },
    /// Start recording the main mix into `Some` recorder, or stop.
    Record(Option<Box<Recorder>>),
    /// Stems for the track on `deck` (`None` removes them). They must match
    /// the track's sample rate and length, or they are dropped.
    SetStems {
        deck: u8,
        track_id: u64,
        stems: Option<Arc<StemAudio>>,
    },
    /// More of a track that is still arriving (streamed while it downloads):
    /// `audio` starts like the deck's track and is longer. `length_secs` is
    /// the whole track's length while more is to come, `None` once `audio`
    /// is all of it. Until then the deck waits at the end of what it has
    /// instead of ending the track. Sent right after [`Command::Load`] with
    /// the same audio, it marks the loaded track as still arriving.
    ExtendTrack {
        deck: u8,
        track_id: u64,
        audio: Arc<TrackAudio>,
        length_secs: Option<f64>,
    },
    /// The drum machine's samples (`None` = silent).
    SetDrumKit(Option<Arc<DrumKit>>),
    /// Replace drum pattern `index` (`0..16`).
    SetDrumPattern {
        index: u8,
        pattern: rille_core::drums::Pattern,
    },
    /// Restore the drum machine's levels, tuning, pattern and channel.
    SetDrumParams(DrumParams),
}

/// Where the audio thread copies every rendered block of the main mix while
/// recording. Frames that do not fit (the reader fell behind) are left out
/// and counted in `dropped`.
pub struct Recorder {
    pub frames: rtrb::Producer<[f32; 2]>,
    pub dropped: Arc<AtomicU64>,
}

impl Recorder {
    /// A recorder and the consumer to read its frames from; `capacity` frames
    /// can wait to be read.
    pub fn new(capacity: usize) -> (Box<Recorder>, rtrb::Consumer<[f32; 2]>, Arc<AtomicU64>) {
        let (frames, consumer) = rtrb::RingBuffer::new(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        (Box::new(Recorder { frames, dropped: dropped.clone() }), consumer, dropped)
    }

    /// Copies `block` (never blocks, never allocates).
    pub fn push(&mut self, block: &[[f32; 2]]) {
        let n = block.len().min(self.frames.slots());
        if let Ok(chunk) = self.frames.write_chunk_uninit(n) {
            chunk.fill_from_iter(block.iter().copied());
        }
        if n < block.len() {
            self.dropped.fetch_add((block.len() - n) as u64, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Things the app must know about, e.g. to persist cue points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    MainCueSet { deck: u8, track_id: u64, secs: f64 },
    HotcueSet { deck: u8, track_id: u64, slot: u8, cue: Hotcue },
    HotcueDeleted { deck: u8, track_id: u64, slot: u8 },
    TrackEnded { deck: u8, track_id: u64 },
}

/// Arcs the audio thread no longer needs, dropped on the app side so the
/// audio thread never frees memory.
#[allow(dead_code)]
pub(crate) enum Garbage {
    Audio(Arc<TrackAudio>),
    Grid(Arc<BeatGrid>),
    Remix(Box<RemixDeck>),
    Sample(RemixSample),
    Recorder(Box<Recorder>),
    Stems(Arc<StemAudio>),
    DrumKit(Arc<DrumKit>),
}
