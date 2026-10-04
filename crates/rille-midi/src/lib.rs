//! MIDI controller support: mapping files, MIDI learn, soft-takeover, jog
//! wheels, LED feedback and device I/O, including HID controllers translated
//! to MIDI.
//!
//! Everything but [`device`] is pure and works without hardware: the app
//! feeds raw bytes into a [`MappingEngine`] and gets [`rille_core::ControlEvent`]s
//! back.

pub mod caiaq;
pub mod device;
pub mod engine;
pub mod feedback;
pub mod hid;
pub mod learn;
pub mod mapping;
pub mod message;
pub mod screen;
pub mod store;

pub use device::{MidiError, MidiEvent, MidiManager, port_base_name};
pub use engine::{MappingEngine, ValueMap, ValueSource};
pub use feedback::FeedbackState;
pub use learn::LearnSession;
pub use mapping::{
    Blink, Condition, Encoding, InputBinding, InputMode, InputTarget, Mapping, MappingError, MidiSpec, OutputBinding,
};
pub use message::MidiMsg;
pub use store::MappingStore;
