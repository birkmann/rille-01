//! Real-time audio engine: decks, beat-locked sync, mixer, effects.
//!
//! [`create`] returns an [`EngineHandle`] for the app and an [`Engine`] for
//! the audio thread. They talk through lock-free queues only: commands in,
//! events and state snapshots out, and memory the audio thread no longer
//! needs goes back to be freed on the app side.

mod deck;
mod engine;
mod mixer;
pub mod remix;
pub mod snapshot;
mod types;
mod voice;

#[cfg(feature = "cpal")]
pub mod backend;
pub mod realtime;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use engine::Engine;
pub use mixer::{band_gains, crossfader_gain, fader_gain, fader_level, gain_knob_db};
pub use remix::{DEFAULT_REMIX_QUANT, REMIX_QUANT_SIZES, RemixDeck, RemixSample, remix_grid};
pub use snapshot::{ChannelState, DeckState, FxState, RemixCellState, RemixSlotState, RemixState, Snapshot};
pub use types::{
    Command, DEFAULT_LOOP_SIZE, Event, FX_UNITS, HOTCUES, Hotcue, LOOP_SIZES, LoadedTrack, MAX_DECKS, Recorder,
    STEM_NAMES, STEMS, Settings, StemAudio, TrackAudio,
};

use types::Garbage;

/// App-side end of the engine. All methods are cheap and non-blocking.
pub struct EngineHandle {
    commands: Mutex<rtrb::Producer<Command>>,
    events: Mutex<rtrb::Consumer<Event>>,
    garbage: Mutex<rtrb::Consumer<Garbage>>,
    snapshot: Mutex<triple_buffer::Output<Snapshot>>,
    /// Buffer underruns/overruns reported by the audio output.
    xruns: Arc<AtomicU64>,
    sample_rate: u32,
    max_block: usize,
}

impl EngineHandle {
    /// A remix deck for deck `deck` of this engine (allocates; send it with
    /// [`Command::SetRemix`]). `id` names it in [`Command::SetGrid`].
    pub fn new_remix_deck(&self, deck: u8, id: u64, bpm: f64) -> Box<RemixDeck> {
        RemixDeck::new(self.sample_rate, self.max_block, deck, id, bpm)
    }

    /// Buffer underruns/overruns (dropouts) since the output started.
    pub fn xruns(&self) -> u64 {
        self.xruns.load(Ordering::Relaxed)
    }

    /// Queues a command; gives it back if the queue is full.
    #[allow(clippy::result_large_err)]
    pub fn send(&self, cmd: Command) -> Result<(), Command> {
        let mut q = self.commands.lock().expect("engine command lock");
        q.push(cmd).map_err(|rtrb::PushError::Full(c)| c)
    }

    /// Latest state published by the audio thread.
    pub fn snapshot(&self) -> Snapshot {
        *self.snapshot.lock().expect("engine snapshot lock").read()
    }

    /// Delivers pending events and frees memory released by the audio thread.
    pub fn poll(&self, mut on_event: impl FnMut(Event)) {
        while let Ok(e) = self.events.lock().expect("engine event lock").pop() {
            on_event(e);
        }
        let mut g = self.garbage.lock().expect("engine garbage lock");
        while let Ok(item) = g.pop() {
            drop(item);
        }
    }
}

/// Creates the engine for `sample_rate`, rendering at most `max_block`
/// frames per internal block.
pub fn create(sample_rate: u32, max_block: usize) -> (EngineHandle, Engine) {
    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::new(4096);
    let (ev_tx, ev_rx) = rtrb::RingBuffer::new(1024);
    let (gb_tx, gb_rx) = rtrb::RingBuffer::new(256);
    let (snap_in, snap_out) = triple_buffer::triple_buffer(&Snapshot::default());
    let engine = Engine::new(
        sample_rate,
        max_block,
        engine::Channels { commands: cmd_rx, events: ev_tx, garbage: gb_tx, snapshot: snap_in },
    );
    let handle = EngineHandle {
        commands: Mutex::new(cmd_tx),
        events: Mutex::new(ev_rx),
        garbage: Mutex::new(gb_rx),
        snapshot: Mutex::new(snap_out),
        xruns: Arc::new(AtomicU64::new(0)),
        sample_rate,
        max_block,
    };
    (handle, engine)
}
