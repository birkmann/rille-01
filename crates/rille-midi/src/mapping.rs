//! Mapping files: which MIDI message drives which control, and which control
//! state lights which LED.
//!
//! Mappings are TOML. Channels are `1..=16` in files. See
//! `mappings/generic-2deck.toml` at the repository root for a commented
//! example.

use regex::Regex;
use rille_core::ids::MAX_DECKS;
use rille_core::{Control, ControlKind, ControlTarget, Scope};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// What an input drives: a control, or a named modifier (`modifier:shift`)
/// that other inputs can use as a [`Condition`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InputTarget {
    Control(ControlTarget),
    Modifier(String),
}

impl InputTarget {
    pub fn control(&self) -> Option<ControlTarget> {
        match self {
            InputTarget::Control(t) => Some(*t),
            InputTarget::Modifier(_) => None,
        }
    }
}

impl From<ControlTarget> for InputTarget {
    fn from(t: ControlTarget) -> Self {
        InputTarget::Control(t)
    }
}

impl fmt::Display for InputTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputTarget::Control(t) => t.fmt(f),
            InputTarget::Modifier(m) => write!(f, "modifier:{m}"),
        }
    }
}

impl FromStr for InputTarget {
    type Err = rille_core::control::ParseControlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.strip_prefix("modifier:") {
            Some(m) if !m.is_empty() => Ok(InputTarget::Modifier(m.to_owned())),
            _ => s.parse().map(InputTarget::Control),
        }
    }
}

impl Serialize for InputTarget {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for InputTarget {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// The MIDI message a binding listens to or sends. `channel` is `1..=16`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MidiSpec {
    Note {
        channel: u8,
        number: u8,
    },
    Cc {
        channel: u8,
        number: u8,
    },
    /// 14-bit CC: MSB on `number` (`0..=31`), LSB on `number + 32`.
    Cc14 {
        channel: u8,
        number: u8,
    },
    #[serde(rename = "pitchbend")]
    PitchBend {
        channel: u8,
    },
}

impl MidiSpec {
    /// Channel as used on the wire, `0..=15`.
    pub fn channel0(self) -> u8 {
        let (MidiSpec::Note { channel, .. }
        | MidiSpec::Cc { channel, .. }
        | MidiSpec::Cc14 { channel, .. }
        | MidiSpec::PitchBend { channel }) = self;
        channel.wrapping_sub(1)
    }

    fn validate(self) -> Result<(), String> {
        let (channel, number, max) = match self {
            MidiSpec::Note { channel, number } | MidiSpec::Cc { channel, number } => (channel, number, 127),
            MidiSpec::Cc14 { channel, number } => (channel, number, 31),
            MidiSpec::PitchBend { channel } => (channel, 0, 0),
        };
        if !(1..=16).contains(&channel) {
            return Err(format!("channel {channel} out of range 1..=16"));
        }
        if number > max {
            return Err(format!("number {number} out of range 0..={max}"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// Press and release → [`rille_core::ControlValue::Press`]. For a CC, any
    /// value above 0 counts as pressed.
    Button,
    /// Fader or knob → [`rille_core::ControlValue::Absolute`] in `min..=max`.
    Absolute,
    /// Endless encoder → [`rille_core::ControlValue::Delta`] of `ticks * step`.
    /// A note input sends one `step` per press. On a continuous control the
    /// engine turns the delta into an absolute value.
    Relative,
    /// Jog wheel → `Delta` in revolutions (`ticks / ticks_per_rev`).
    Jog,
}

/// How a relative CC encodes signed ticks in its 7-bit value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    /// 1 = +1, 127 = −1.
    #[default]
    TwosComplement,
    /// 65 = +1, 63 = −1.
    BinaryOffset,
    /// 1 = +1, 65 = −1.
    SignMagnitude,
}

impl Encoding {
    pub fn decode(self, v: u8) -> i32 {
        let v = i32::from(v & 0x7f);
        match self {
            Encoding::TwosComplement if v >= 64 => v - 128,
            Encoding::TwosComplement => v,
            Encoding::BinaryOffset => v - 64,
            Encoding::SignMagnitude if v & 0x40 != 0 => -(v & 0x3f),
            Encoding::SignMagnitude => v,
        }
    }
}

/// Only apply the binding while a modifier has the given state.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub modifier: String,
    #[serde(default = "yes")]
    pub value: bool,
}

fn yes() -> bool {
    true
}
fn is_false(b: &bool) -> bool {
    !*b
}
fn zero() -> f32 {
    0.0
}
fn one() -> f32 {
    1.0
}
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn is_one(v: &f32) -> bool {
    *v == 1.0
}

/// Default jog resolution when a mapping gives none.
pub const DEFAULT_TICKS_PER_REV: f32 = 128.0;
/// Default relative step on continuous controls (fraction of the range per tick).
pub const DEFAULT_CONTINUOUS_STEP: f32 = 0.01;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputBinding {
    pub target: InputTarget,
    pub midi: MidiSpec,
    /// Inferred from the target when omitted, see [`InputBinding::mode`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<InputMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<Encoding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks_per_rev: Option<f32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert: bool,
    /// Absolute inputs only; defaults to on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_takeover: Option<bool>,
    #[serde(default = "zero", skip_serializing_if = "is_zero")]
    pub min: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub max: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
    /// Modifier inputs only: each press toggles the modifier instead of
    /// holding it (a mode button such as TOUCH).
    #[serde(default, skip_serializing_if = "is_false")]
    pub latch: bool,
}

impl InputBinding {
    pub fn new(target: impl Into<InputTarget>, midi: MidiSpec) -> Self {
        Self {
            target: target.into(),
            midi,
            mode: None,
            encoding: None,
            step: None,
            ticks_per_rev: None,
            invert: false,
            soft_takeover: None,
            min: 0.0,
            max: 1.0,
            condition: None,
            latch: false,
        }
    }

    pub fn with_mode(mut self, mode: InputMode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Explicit mode, else: modifiers and buttons → button, continuous →
    /// absolute, `jog` → jog, other relative controls → relative.
    pub fn mode(&self) -> InputMode {
        self.mode.unwrap_or(match &self.target {
            InputTarget::Modifier(_) => InputMode::Button,
            InputTarget::Control(t) if t.control == Control::Jog => InputMode::Jog,
            InputTarget::Control(t) => match t.control.kind() {
                ControlKind::Button => InputMode::Button,
                ControlKind::Continuous => InputMode::Absolute,
                ControlKind::Relative => InputMode::Relative,
            },
        })
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding.unwrap_or_default()
    }

    pub fn step(&self) -> f32 {
        let continuous = self.target.control().is_some_and(|t| t.control.kind() == ControlKind::Continuous);
        self.step.unwrap_or(if continuous { DEFAULT_CONTINUOUS_STEP } else { 1.0 })
    }

    pub fn ticks_per_rev(&self) -> f32 {
        self.ticks_per_rev.unwrap_or(DEFAULT_TICKS_PER_REV)
    }

    pub fn soft_takeover(&self) -> bool {
        self.soft_takeover.unwrap_or(true)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.midi.validate()?;
        let kind = self.target.control().map(|t| t.control.kind());
        let mode = self.mode();
        let midi_ok = match (mode, self.midi) {
            (InputMode::Button, MidiSpec::Note { .. } | MidiSpec::Cc { .. }) => true,
            (InputMode::Absolute, m) => !matches!(m, MidiSpec::Note { .. }),
            (InputMode::Relative, m) => !matches!(m, MidiSpec::Cc14 { .. }),
            (InputMode::Jog, m) => matches!(m, MidiSpec::Cc { .. } | MidiSpec::PitchBend { .. }),
            _ => false,
        };
        if !midi_ok {
            return Err(format!("{mode:?} mode cannot use {:?} messages", self.midi));
        }
        let target_ok = match mode {
            InputMode::Button => matches!(kind, None | Some(ControlKind::Button)),
            InputMode::Absolute => kind == Some(ControlKind::Continuous),
            InputMode::Relative => matches!(kind, Some(ControlKind::Relative | ControlKind::Continuous)),
            InputMode::Jog => kind == Some(ControlKind::Relative),
        };
        if !target_ok {
            return Err(format!("{mode:?} mode does not fit target {}", self.target));
        }
        if self.target.control().is_some_and(|t| !t.control.is_input()) {
            return Err(format!("{} is read-only; use it as an output source", self.target));
        }
        let finite = [self.min, self.max, self.step(), self.ticks_per_rev()].iter().all(|v| v.is_finite());
        if !finite || self.ticks_per_rev() <= 0.0 {
            return Err("min, max, step and ticks_per_rev must be finite, ticks_per_rev positive".into());
        }
        if self.condition.as_ref().is_some_and(|c| c.modifier.is_empty()) {
            return Err("condition needs a modifier name".into());
        }
        if self.latch && !matches!(self.target, InputTarget::Modifier(_)) {
            return Err("latch is for modifier inputs only".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blink {
    /// While the state is on, lit for the first half of every beat.
    Beat,
}

fn default_on() -> u8 {
    127
}
fn is_default_on(v: &u8) -> bool {
    *v == 127
}
fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

/// LED feedback. Button sources and modifiers (`modifier:touch`) send `on`
/// when their value is above 0.5 (the modifier is on), else `off`;
/// continuous sources send `off + value * (on - off)`. With a `threshold`,
/// any source sends `on` once its value reaches it, else `off`, e.g. one LED
/// of a level meter. With `raw`, the source value itself is sent, rounded and
/// clamped to `0..=127`, e.g. a remix pad's LED code or a page number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputBinding {
    pub source: InputTarget,
    pub midi: MidiSpec,
    #[serde(default = "default_on", skip_serializing_if = "is_default_on")]
    pub on: u8,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub off: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blink: Option<Blink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    /// Level meters: the source is a linear amplitude, shown in dB from this
    /// floor (e.g. -60) at 0 up to 0 dBFS at 1, before `threshold` or the
    /// `off`..`on` scaling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_floor: Option<f32>,
    /// Send the value as is, ignoring `on` and `blink`; `off` is still what
    /// switching every output off sends.
    #[serde(default, skip_serializing_if = "is_false")]
    pub raw: bool,
    /// Only shown while a modifier has the given state; it then takes the
    /// place of the outputs without a condition on the same message (e.g.
    /// a display shows the capture source while CAPTURE is held).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
}

impl OutputBinding {
    pub fn validate(&self) -> Result<(), String> {
        self.midi.validate()?;
        if !matches!(self.midi, MidiSpec::Note { .. } | MidiSpec::Cc { .. }) {
            return Err("outputs support note and cc messages only".into());
        }
        if self.on > 127 || self.off > 127 {
            return Err("on/off values must be 0..=127".into());
        }
        if self.threshold.is_some_and(|t| !t.is_finite()) {
            return Err("threshold must be a finite number".into());
        }
        if self.db_floor.is_some_and(|f| !(f.is_finite() && f < 0.0)) {
            return Err("db_floor must be a negative number of dB".into());
        }
        if self.raw && (self.threshold.is_some() || self.db_floor.is_some() || self.blink.is_some()) {
            return Err("raw cannot be combined with threshold, db_floor or blink".into());
        }
        if self.condition.as_ref().is_some_and(|c| c.modifier.is_empty()) {
            return Err("condition needs a modifier name".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub name: String,
    /// Regex matched against the MIDI port name.
    pub device: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Deck orders the mapping can run in, e.g. `["CABD", "ABCD"]`. The file
    /// then uses decks A, B, C, … for the controller's first, second, third …
    /// deck section, and each layout names the app deck every section drives,
    /// left to right. The store lists one mapping per layout, see
    /// [`Mapping::variants`]; the first is the default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deck_layouts: Vec<String>,
    /// Names of other mappings whose bindings this one adds, as written in
    /// their files (without a deck layout suffix): a controller that sends
    /// its MIDI through another device's port, such as a Xone:K2 on a
    /// Xone:96's X:LINK. Their deck sections follow this mapping's deck
    /// layouts. The store resolves includes when it loads, one level deep.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, rename = "input", skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<InputBinding>,
    #[serde(default, rename = "output", skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<OutputBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MappingError {
    Io(String),
    Parse(String),
    Regex(String),
    Layout(String),
    Input { index: usize, msg: String },
    Output { index: usize, msg: String },
}

impl fmt::Display for MappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot read mapping: {e}"),
            Self::Parse(e) => write!(f, "invalid mapping file: {e}"),
            Self::Regex(e) => write!(f, "invalid device regex: {e}"),
            Self::Layout(e) => write!(f, "{e}"),
            Self::Input { index, msg } => write!(f, "input #{}: {msg}", index + 1),
            Self::Output { index, msg } => write!(f, "output #{}: {msg}", index + 1),
        }
    }
}

impl std::error::Error for MappingError {}

impl Mapping {
    pub fn new(name: impl Into<String>, device: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            device: device.into(),
            description: String::new(),
            deck_layouts: Vec::new(),
            include: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    /// Parses and validates.
    pub fn from_toml(s: &str) -> Result<Self, MappingError> {
        let m: Mapping = toml::from_str(s).map_err(|e| MappingError::Parse(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("mapping serializes")
    }

    pub fn validate(&self) -> Result<(), MappingError> {
        self.device_regex()?;
        let mut sections = MAX_DECKS;
        for layout in &self.deck_layouts {
            sections = sections.min(parse_layout(layout).map_err(MappingError::Layout)?.len());
        }
        let deck_ok = |t: &ControlTarget| t.control.scope() != Scope::Deck || usize::from(t.unit) < sections;
        let no_deck = |t: &ControlTarget| format!("{t}: no deck {} in every deck layout", char::from(b'A' + t.unit));
        for (index, b) in self.inputs.iter().enumerate() {
            b.validate().map_err(|msg| MappingError::Input { index, msg: format!("{}: {msg}", b.target) })?;
            if let Some(t) = b.target.control().filter(|t| !deck_ok(t)) {
                return Err(MappingError::Input { index, msg: no_deck(&t) });
            }
        }
        for (index, b) in self.outputs.iter().enumerate() {
            b.validate().map_err(|msg| MappingError::Output { index, msg: format!("{}: {msg}", b.source) })?;
            if let Some(t) = b.source.control().filter(|t| !deck_ok(t)) {
                return Err(MappingError::Output { index, msg: no_deck(&t) });
            }
        }
        Ok(())
    }

    /// One mapping per deck layout, named `"<name> (<layout>)"`, with deck
    /// targets moved to the layout's decks. Without layouts, just a copy.
    /// Invalid layouts are skipped; validate the mapping first to report them.
    pub fn variants(&self) -> Vec<Mapping> {
        if self.deck_layouts.is_empty() {
            return vec![self.clone()];
        }
        self.deck_layouts.iter().filter_map(|l| self.with_deck_layout(l).ok()).collect()
    }

    /// This mapping with deck section `i` driving deck `layout[i]`.
    pub fn with_deck_layout(&self, layout: &str) -> Result<Mapping, MappingError> {
        let order = parse_layout(layout).map_err(MappingError::Layout)?;
        let move_deck = |t: &mut ControlTarget| {
            if t.control.scope() == Scope::Deck {
                t.unit = order.get(usize::from(t.unit)).copied().unwrap_or(t.unit);
            }
        };
        let mut m = self.clone();
        m.name = format!("{} ({layout})", self.name);
        m.deck_layouts.clear();
        let targets = m.inputs.iter_mut().map(|b| &mut b.target).chain(m.outputs.iter_mut().map(|b| &mut b.source));
        for t in targets {
            if let InputTarget::Control(t) = t {
                move_deck(t);
            }
        }
        Ok(m)
    }

    pub fn device_regex(&self) -> Result<Regex, MappingError> {
        Regex::new(&self.device).map_err(|e| MappingError::Regex(e.to_string()))
    }

    /// Replaces any binding for the same target and condition, e.g. after
    /// MIDI learn.
    pub fn set_input(&mut self, binding: InputBinding) {
        self.inputs.retain(|b| b.target != binding.target || b.condition != binding.condition);
        self.inputs.push(binding);
    }
}

/// `"CABD"` → deck indices `[2, 0, 1, 3]`: distinct letters `A..=D`.
fn parse_layout(layout: &str) -> Result<Vec<u8>, String> {
    let err = || format!("deck layout '{layout}' must list distinct decks A-D, e.g. \"CABD\"");
    let order: Vec<u8> = layout
        .chars()
        .map(|c| c.is_ascii_uppercase().then(|| c as u8 - b'A').filter(|&d| usize::from(d) < MAX_DECKS))
        .collect::<Option<_>>()
        .ok_or_else(err)?;
    let distinct = order.iter().enumerate().all(|(i, d)| !order[..i].contains(d));
    if order.is_empty() || !distinct {
        return Err(err());
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
        name = "Test"
        device = "(?i)generic"

        [[input]]
        target = "deck.A.play"
        midi = { type = "note", channel = 1, number = 11 }

        [[input]]
        target = "deck.A.volume"
        midi = { type = "cc14", channel = 1, number = 19 }
        soft_takeover = false

        [[input]]
        target = "deck.A.jog"
        midi = { type = "cc", channel = 1, number = 34 }
        encoding = "binary_offset"
        ticks_per_rev = 720

        [[input]]
        target = "modifier:shift"
        midi = { type = "note", channel = 1, number = 63 }

        [[input]]
        target = "deck.A.hotcue_delete.1"
        midi = { type = "note", channel = 8, number = 0 }
        condition = { modifier = "shift" }

        [[input]]
        target = "global.crossfader"
        midi = { type = "pitchbend", channel = 3 }
        invert = true

        [[output]]
        source = "deck.A.play"
        midi = { type = "note", channel = 1, number = 11 }
        blink = "beat"
    "#;

    #[test]
    fn parses_example_with_defaults() {
        let m = Mapping::from_toml(EXAMPLE).unwrap();
        assert_eq!(m.inputs.len(), 6);
        let [play, vol, jog, shift, del, xf] = &m.inputs[..] else { panic!() };
        assert_eq!(play.mode(), InputMode::Button);
        assert_eq!(play.midi.channel0(), 0);
        assert_eq!(vol.mode(), InputMode::Absolute);
        assert!(!vol.soft_takeover());
        assert_eq!(jog.mode(), InputMode::Jog);
        assert_eq!(jog.encoding(), Encoding::BinaryOffset);
        assert_eq!(jog.ticks_per_rev(), 720.0);
        assert_eq!(shift.target, InputTarget::Modifier("shift".into()));
        assert_eq!(del.condition, Some(Condition { modifier: "shift".into(), value: true }));
        assert!(xf.invert && xf.soft_takeover());
        assert_eq!(m.outputs[0].on, 127);
        assert_eq!(m.outputs[0].blink, Some(Blink::Beat));
    }

    #[test]
    fn toml_roundtrip() {
        let m = Mapping::from_toml(EXAMPLE).unwrap();
        let again = Mapping::from_toml(&m.to_toml()).unwrap();
        assert_eq!(m, again);
    }

    fn err(input: &str) -> MappingError {
        let src = format!("name = \"x\"\ndevice = \"x\"\n[[input]]\n{input}");
        Mapping::from_toml(&src).unwrap_err()
    }

    #[test]
    fn validation_errors() {
        let msg = |e: MappingError| e.to_string();
        assert!(matches!(err(r#"target = "deck.Z.play""#), MappingError::Parse(_)));
        assert!(matches!(err(r#"target = "deck.A.play""#), MappingError::Parse(_))); // missing midi
        let e = err(r#"target = "deck.A.play"
                     midi = { type = "note", channel = 0, number = 1 }"#);
        assert!(msg(e).contains("channel 0"));
        let e = err(r#"target = "deck.A.play"
                     midi = { type = "note", channel = 1, number = 128 }"#);
        assert!(matches!(e, MappingError::Input { index: 0, .. }));
        let e = err(r#"target = "deck.A.volume"
                     midi = { type = "cc14", channel = 1, number = 40 }"#);
        assert!(msg(e).contains("0..=31"));
        let e = err(r#"target = "deck.A.play"
                     midi = { type = "cc", channel = 1, number = 1 }
                     mode = "absolute""#);
        assert!(msg(e).contains("does not fit"));
        let e = err(r#"target = "deck.A.volume"
                     midi = { type = "note", channel = 1, number = 1 }"#);
        assert!(msg(e).contains("cannot use"));
        let e = err(r#"target = "deck.A.meter"
                     midi = { type = "cc", channel = 1, number = 1 }"#);
        assert!(msg(e).contains("read-only"));
        let e = err(r#"target = "deck.A.jog"
                     midi = { type = "cc", channel = 1, number = 1 }
                     ticks_per_rev = 0"#);
        assert!(msg(e).contains("ticks_per_rev"));
        assert!(matches!(
            err(r#"target = "deck.A.play"
                   midi = { type = "note", channel = 1, number = 1 }
                   typo = 1"#),
            MappingError::Parse(_)
        ));
        let bad_regex = "name = \"x\"\ndevice = \"(\"";
        assert!(matches!(Mapping::from_toml(bad_regex), Err(MappingError::Regex(_))));
        let bad_output = "name = \"x\"\ndevice = \"x\"\n[[output]]\nsource = \"deck.A.play\"\n\
                          midi = { type = \"pitchbend\", channel = 1 }";
        assert!(matches!(Mapping::from_toml(bad_output), Err(MappingError::Output { index: 0, .. })));
    }

    #[test]
    fn deck_layouts_move_deck_targets() {
        let src = r#"
            name = "K"
            device = "x"
            deck_layouts = ["CABD", "ABCD"]
            [[input]]
            target = "deck.A.play"
            midi = { type = "note", channel = 1, number = 1 }
            [[input]]
            target = "deck.D.play"
            midi = { type = "note", channel = 1, number = 4 }
            [[input]]
            target = "global.scroll"
            midi = { type = "cc", channel = 1, number = 5 }
            [[output]]
            source = "deck.B.hotcue.1"
            midi = { type = "note", channel = 1, number = 2 }
        "#;
        let m = Mapping::from_toml(src).unwrap();
        assert_eq!(Mapping::from_toml(&m.to_toml()).unwrap(), m);
        let v = m.variants();
        let names: Vec<_> = v.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["K (CABD)", "K (ABCD)"]);
        let targets = |m: &Mapping| m.inputs.iter().map(|b| b.target.to_string()).collect::<Vec<_>>();
        assert_eq!(targets(&v[0]), ["deck.C.play", "deck.D.play", "global.scroll"]);
        assert_eq!(targets(&v[1]), ["deck.A.play", "deck.D.play", "global.scroll"]);
        assert_eq!(v[0].outputs[0].source.to_string(), "deck.A.hotcue.1");
        assert!(v.iter().all(|m| m.deck_layouts.is_empty() && m.validate().is_ok()));
        assert_eq!(Mapping::new("plain", "x").variants()[0].name, "plain");

        // A 2-deck controller driving decks C and D.
        let two = src.replace(r#"["CABD", "ABCD"]"#, r#"["CD"]"#);
        assert!(Mapping::from_toml(&two).unwrap_err().to_string().contains("no deck D"));
        let two = two.replace("deck.D.play", "deck.B.eject");
        assert_eq!(targets(&Mapping::from_toml(&two).unwrap().variants()[0])[..2], ["deck.C.play", "deck.D.eject"]);

        for bad in ["", "AA", "ABCE", "abcd", "A B"] {
            let src = src.replace(r#"["CABD", "ABCD"]"#, &format!("[{bad:?}]"));
            assert!(matches!(Mapping::from_toml(&src), Err(MappingError::Layout(_))), "{bad:?}");
        }
    }

    #[test]
    fn encodings() {
        use Encoding::*;
        for (enc, plus, minus) in [(TwosComplement, 1, 127), (BinaryOffset, 65, 63), (SignMagnitude, 1, 65)] {
            assert_eq!(enc.decode(plus), 1, "{enc:?}");
            assert_eq!(enc.decode(minus), -1, "{enc:?}");
        }
        assert_eq!(TwosComplement.decode(120), -8);
        assert_eq!(BinaryOffset.decode(56), -8);
        assert_eq!(SignMagnitude.decode(72), -8);
    }
}
