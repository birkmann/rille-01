//! Turns raw MIDI into [`ControlEvent`]s according to a [`Mapping`].
//!
//! Pure and allocation-free per message: the caller owns the output vector
//! and passes the time, so everything here is unit-testable without devices.

use crate::mapping::{InputBinding, InputMode, InputTarget, Mapping, MidiSpec};
use crate::message::MidiMsg;
use rille_core::{ControlEvent, ControlKind, ControlTarget, ControlValue};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Current application state, read for soft-takeover, relative knobs and
/// LED feedback.
pub trait ValueSource {
    /// `0..=1` for continuous controls, 0 or 1 for button states (playing,
    /// hotcue set, sync on, …).
    fn value(&self, target: ControlTarget) -> f32;
    /// Phase within the current master beat, `0..1`.
    fn beat_phase(&self) -> f64 {
        0.0
    }
}

/// A [`ValueSource`] backed by a map; unset targets report their default.
#[derive(Clone, Debug, Default)]
pub struct ValueMap {
    pub values: HashMap<ControlTarget, f32>,
    pub beat_phase: f64,
}

impl ValueMap {
    pub fn set(&mut self, target: ControlTarget, v: f32) {
        self.values.insert(target, v);
    }
}

impl ValueSource for ValueMap {
    fn value(&self, target: ControlTarget) -> f32 {
        self.values.get(&target).copied().unwrap_or_else(|| target.control.default_value())
    }
    fn beat_phase(&self) -> f64 {
        self.beat_phase
    }
}

/// Soft-takeover picks up once the hardware is within this distance of the
/// software value (or crosses it).
pub const PICKUP_DISTANCE: f32 = 0.03;
/// How long the first half of a 14-bit CC pair waits for the other half.
pub const CC14_WINDOW: Duration = Duration::from_millis(20);

/// Values at most this far apart count as equal when comparing with what the
/// app reports back.
const SAME: f32 = 1e-3;

/// The last few values we sent. The app applies them asynchronously, so its
/// reported value may lag a few messages behind; any of these counts as "ours".
#[derive(Clone, Debug, Default)]
struct Recent {
    vals: [f32; 8],
    len: usize,
    pos: usize,
}

impl Recent {
    fn push(&mut self, v: f32) {
        self.vals[self.pos] = v;
        self.pos = (self.pos + 1) % self.vals.len();
        self.len = (self.len + 1).min(self.vals.len());
    }
    fn contains(&self, v: f32) -> bool {
        self.vals[..self.len].iter().any(|x| (x - v).abs() <= SAME)
    }
    fn last(&self) -> Option<f32> {
        (self.len > 0).then(|| self.vals[(self.pos + self.vals.len() - 1) % self.vals.len()])
    }
    fn clear(&mut self) {
        self.len = 0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Half {
    Msb,
    Lsb,
}

/// Per-binding runtime state.
#[derive(Clone, Debug, Default)]
struct Slot {
    /// Button: a press was sent and its release is outstanding.
    pressed: bool,
    // 14-bit CC
    msb: u8,
    lsb: u8,
    lsb_seen: bool,
    pending: Option<(Half, Instant)>,
    // Soft-takeover / relative-on-continuous
    engaged: bool,
    last_hw: Option<f32>,
    sent: Recent,
}

impl Slot {
    /// Soft-takeover: returns whether `hw` should be sent.
    fn take_over(&mut self, hw: f32, sw: f32) -> bool {
        if !self.sent.contains(sw) {
            // Changed on the software side (or never sent): wait for pickup.
            self.engaged = false;
            self.sent.clear();
        }
        if !self.engaged {
            let crossed = self.last_hw.is_some_and(|p| (p - sw) * (hw - sw) <= 0.0);
            if crossed || (hw - sw).abs() <= PICKUP_DISTANCE {
                self.engaged = true;
                self.sent.push(sw);
            }
        }
        self.last_hw = Some(hw);
        if self.engaged {
            self.sent.push(hw);
        }
        self.engaged
    }

    fn value14(&self) -> f32 {
        f32::from(u16::from(self.msb) << 7 | u16::from(self.lsb)) / 16383.0
    }
}

/// Decoded input for one binding.
#[derive(Clone, Copy, Debug)]
enum Input {
    Note(bool),
    Cc(u8),
    Half(Half, u8),
    Bend(u16),
}

fn key(kind: u8, ch: u8, num: u8) -> u32 {
    u32::from(kind) << 16 | u32::from(ch) << 8 | u32::from(num)
}
const NOTE: u8 = 0;
const CC: u8 = 1;
const BEND: u8 = 2;

pub struct MappingEngine {
    mapping: Mapping,
    /// Message key → bindings listening to it (with the 14-bit half they get).
    index: HashMap<u32, Vec<(usize, Option<Half>)>>,
    slots: Vec<Slot>,
    modifiers: Vec<(String, bool)>,
    /// Modifiers toggled per press (`latch`) rather than held.
    latched: Vec<String>,
    /// A `deck_layout:next` input was pressed, see
    /// [`take_next_deck_layout`](Self::take_next_deck_layout).
    next_layout: bool,
}

impl MappingEngine {
    /// Invalid bindings are skipped; validate the mapping first to report them.
    pub fn new(mapping: Mapping) -> Self {
        let mut index: HashMap<u32, Vec<_>> = HashMap::new();
        for (i, b) in mapping.inputs.iter().enumerate().filter(|(_, b)| b.validate().is_ok()) {
            let ch = b.midi.channel0();
            let mut add = |k, half| index.entry(k).or_default().push((i, half));
            match b.midi {
                MidiSpec::Note { number, .. } => add(key(NOTE, ch, number), None),
                MidiSpec::Cc { number, .. } => add(key(CC, ch, number), None),
                MidiSpec::Cc14 { number, .. } => {
                    add(key(CC, ch, number), Some(Half::Msb));
                    add(key(CC, ch, number + 32), Some(Half::Lsb));
                }
                MidiSpec::PitchBend { .. } => add(key(BEND, ch, 0), None),
            }
        }
        let slots = vec![Slot::default(); mapping.inputs.len()];
        let latched = mapping
            .inputs
            .iter()
            .filter(|b| b.latch)
            .filter_map(|b| match &b.target {
                InputTarget::Modifier(m) => Some(m.clone()),
                _ => None,
            })
            .collect();
        Self { mapping, index, slots, modifiers: Vec::new(), latched, next_layout: false }
    }

    pub fn mapping(&self) -> &Mapping {
        &self.mapping
    }

    pub fn modifier(&self, name: &str) -> bool {
        self.modifiers.iter().any(|(n, v)| n == name && *v)
    }

    /// Whether a `deck_layout:next` input was pressed since the last call:
    /// the controller asks for its mapping's next deck layout, which only the
    /// caller can switch to.
    pub fn take_next_deck_layout(&mut self) -> bool {
        std::mem::take(&mut self.next_layout)
    }

    /// Takes over the modifiers of `previous`, the engine this one replaces
    /// on the same device, so a SHIFT held while the mapping changes stays
    /// held.
    pub fn keep_modifiers(&mut self, previous: &MappingEngine) {
        self.modifiers.clone_from(&previous.modifiers);
    }

    /// Forgets pickup, button and 14-bit state.
    pub fn reset(&mut self) {
        self.slots.iter_mut().for_each(|s| *s = Slot::default());
        self.modifiers.clear();
    }

    /// Handles one raw MIDI message received at `t`, appending events to `out`.
    ///
    /// When several bindings listen to the same message, those with a
    /// matching `condition` take precedence over unconditional ones, so
    /// "shift + play" can be mapped next to "play", and among those, the ones
    /// on a held modifier over the ones on a latched mode (`latch`), so a
    /// held SHIFT still works in a mode. Releases always go to the binding
    /// that received the press.
    pub fn handle(&mut self, raw: &[u8], t: Instant, values: &dyn ValueSource, out: &mut Vec<ControlEvent>) {
        self.flush(t, values, out);
        let (k, input) = match MidiMsg::parse(raw) {
            MidiMsg::NoteOn { ch, note, .. } => (key(NOTE, ch, note), Input::Note(true)),
            MidiMsg::NoteOff { ch, note, .. } => (key(NOTE, ch, note), Input::Note(false)),
            MidiMsg::Cc { ch, num, val } => (key(CC, ch, num), Input::Cc(val)),
            MidiMsg::PitchBend { ch, val14 } => (key(BEND, ch, 0), Input::Bend(val14)),
            MidiMsg::Other => return,
        };
        let Some(list) = self.index.get(&k) else { return };
        let inputs = &self.mapping.inputs;
        let on_latch = |b: &InputBinding| b.condition.as_ref().is_some_and(|c| self.latched.contains(&c.modifier));
        let matches = |b: &InputBinding| b.condition.is_some() && condition_holds(&self.modifiers, b);
        let conditioned_match = list.iter().any(|&(i, _)| matches(&inputs[i]));
        let held_match = list.iter().any(|&(i, _)| matches(&inputs[i]) && !on_latch(&inputs[i]));
        for &(i, half) in list {
            let b = &inputs[i];
            let outranked = if b.condition.is_some() { held_match && on_latch(b) } else { conditioned_match };
            let active = condition_holds(&self.modifiers, b) && !outranked;
            let input = match (half, input) {
                (Some(h), Input::Cc(v)) => Input::Half(h, v),
                _ => input,
            };
            let mut st = State {
                modifiers: &mut self.modifiers,
                next_layout: &mut self.next_layout,
                slot: &mut self.slots[i],
                values,
                out,
            };
            st.apply(b, input, active, t);
        }
    }

    /// Emits 14-bit values whose second half did not arrive within
    /// [`CC14_WINDOW`]. `handle` does this too; call it periodically so a
    /// lone half is not held back until the next message.
    pub fn flush(&mut self, t: Instant, values: &dyn ValueSource, out: &mut Vec<ControlEvent>) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.pending.is_some_and(|(_, t0)| t.saturating_duration_since(t0) > CC14_WINDOW) {
                slot.pending = None;
                let b = &self.mapping.inputs[i];
                let active = condition_holds(&self.modifiers, b);
                let v = slot.value14();
                let next_layout = &mut self.next_layout;
                State { modifiers: &mut self.modifiers, next_layout, slot, values, out }.absolute(b, v, active);
            }
        }
    }
}

fn condition_holds(modifiers: &[(String, bool)], b: &InputBinding) -> bool {
    b.condition.as_ref().is_none_or(|c| modifiers.iter().any(|(n, v)| *n == c.modifier && *v) == c.value)
}

struct State<'a> {
    modifiers: &'a mut Vec<(String, bool)>,
    next_layout: &'a mut bool,
    slot: &'a mut Slot,
    values: &'a dyn ValueSource,
    out: &'a mut Vec<ControlEvent>,
}

impl State<'_> {
    fn apply(&mut self, b: &InputBinding, input: Input, active: bool, t: Instant) {
        match b.mode() {
            InputMode::Button => {
                let down = match input {
                    Input::Note(down) => down,
                    Input::Cc(v) => v > 0,
                    _ => return,
                };
                if down && active && !self.slot.pressed {
                    self.slot.pressed = true;
                    self.press(b, true);
                } else if !down && self.slot.pressed {
                    self.slot.pressed = false;
                    self.press(b, false);
                }
            }
            InputMode::Absolute => match input {
                Input::Cc(v) => self.absolute(b, f32::from(v) / 127.0, active),
                Input::Bend(v) => self.absolute(b, f32::from(v) / 16383.0, active),
                Input::Half(half, v) => self.half(b, half, v, t, active),
                Input::Note(_) => {}
            },
            InputMode::Relative | InputMode::Jog => {
                let ticks = match input {
                    Input::Note(true) => 1,
                    Input::Cc(v) => b.encoding().decode(v),
                    Input::Bend(v) => i32::from(v) - 8192,
                    _ => 0,
                };
                if ticks != 0 && active {
                    self.relative(b, ticks as f32);
                }
            }
        }
    }

    fn press(&mut self, b: &InputBinding, down: bool) {
        match &b.target {
            InputTarget::Control(target) => {
                self.out.push(ControlEvent { target: *target, value: ControlValue::Press(down) });
            }
            InputTarget::Modifier(name) => {
                let i = self.modifiers.iter().position(|(n, _)| n == name).unwrap_or_else(|| {
                    self.modifiers.push((name.clone(), false));
                    self.modifiers.len() - 1
                });
                let on = &mut self.modifiers[i].1;
                if !b.latch {
                    *on = down;
                } else if down {
                    *on = !*on;
                }
            }
            InputTarget::NextDeckLayout => *self.next_layout |= down,
        }
    }

    /// One half of a 14-bit pair. The first half waits for the second; a
    /// controller that never sends LSBs gets plain 7-bit values.
    fn half(&mut self, b: &InputBinding, half: Half, v: u8, t: Instant, active: bool) {
        let s = &mut *self.slot;
        let first_lsb = half == Half::Lsb && !s.lsb_seen;
        match half {
            Half::Msb => s.msb = v,
            Half::Lsb => {
                s.lsb = v;
                s.lsb_seen = true;
            }
        }
        if !s.lsb_seen {
            return self.absolute(b, f32::from(v) / 127.0, active);
        }
        match s.pending {
            _ if first_lsb => {
                let v = s.value14();
                self.absolute(b, v, active);
            }
            Some((h, _)) if h != half => {
                s.pending = None;
                let v = s.value14();
                self.absolute(b, v, active);
            }
            _ => s.pending = Some((half, t)),
        }
    }

    /// `raw` is the normalized hardware position `0..=1`.
    fn absolute(&mut self, b: &InputBinding, raw: f32, active: bool) {
        let InputTarget::Control(target) = b.target else { return };
        if !active {
            return;
        }
        let x = if b.invert { 1.0 - raw } else { raw };
        let v = b.min + x * (b.max - b.min);
        if b.soft_takeover() && !self.slot.take_over(v, self.values.value(target)) {
            return;
        }
        self.out.push(ControlEvent { target, value: ControlValue::Absolute(v) });
    }

    fn relative(&mut self, b: &InputBinding, ticks: f32) {
        let InputTarget::Control(target) = b.target else { return };
        let sign = if b.invert { -1.0 } else { 1.0 };
        let value = match b.mode() {
            InputMode::Jog => ControlValue::Delta(sign * ticks / b.ticks_per_rev()),
            _ if target.control.kind() == ControlKind::Continuous => {
                // Endless knob on a continuous control: step from the value we
                // last sent unless the app changed it since.
                let sw = self.values.value(target);
                if !self.slot.sent.contains(sw) {
                    self.slot.sent.clear();
                    self.slot.sent.push(sw);
                }
                let base = self.slot.sent.last().unwrap_or(sw);
                let (lo, hi) = (b.min.min(b.max), b.min.max(b.max));
                let v = (base + sign * ticks * b.step()).clamp(lo, hi);
                self.slot.sent.push(v);
                ControlValue::Absolute(v)
            }
            _ => ControlValue::Delta(sign * ticks * b.step()),
        };
        self.out.push(ControlEvent { target, value });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{Condition, Encoding};
    use rille_core::Control;

    fn deck_a(c: Control) -> ControlTarget {
        ControlTarget::deck(0, c)
    }

    struct Rig {
        engine: MappingEngine,
        values: ValueMap,
        t: Instant,
    }

    impl Rig {
        fn new(inputs: Vec<InputBinding>) -> Self {
            let mut m = Mapping::new("t", ".*");
            m.inputs = inputs;
            m.validate().unwrap();
            Self { engine: MappingEngine::new(m), values: ValueMap::default(), t: Instant::now() }
        }

        fn send(&mut self, raw: &[u8]) -> Vec<ControlEvent> {
            self.t += Duration::from_millis(1);
            let mut out = Vec::new();
            self.engine.handle(raw, self.t, &self.values, &mut out);
            // Apply absolute values like the app would.
            for e in &out {
                if let ControlValue::Absolute(v) = e.value {
                    self.values.set(e.target, v);
                }
            }
            out
        }

        fn values(&mut self, raw: &[u8]) -> Vec<ControlValue> {
            self.send(raw).into_iter().map(|e| e.value).collect()
        }
    }

    fn assert_close(vals: Vec<ControlValue>, want: f32) {
        match vals[..] {
            [ControlValue::Absolute(v)] if (v - want).abs() < 1e-5 => {}
            _ => panic!("{vals:?} != [Absolute({want})]"),
        }
    }

    fn note(ch: u8, number: u8) -> MidiSpec {
        MidiSpec::Note { channel: ch, number }
    }
    fn cc(ch: u8, number: u8) -> MidiSpec {
        MidiSpec::Cc { channel: ch, number }
    }

    #[test]
    fn button_note_and_cc() {
        let mut r = Rig::new(vec![
            InputBinding::new(deck_a(Control::Play), note(1, 11)),
            InputBinding::new(deck_a(Control::Cue), cc(1, 12)),
        ]);
        let play = deck_a(Control::Play);
        assert_eq!(r.send(&[0x90, 11, 127]), [ControlEvent { target: play, value: ControlValue::Press(true) }]);
        assert_eq!(r.values(&[0x90, 11, 0]), [ControlValue::Press(false)]);
        assert_eq!(r.values(&[0x80, 11, 0]), []); // no press outstanding
        assert!(r.send(&[0x91, 11, 127]).is_empty()); // other channel
        assert_eq!(r.values(&[0xb0, 12, 127]), [ControlValue::Press(true)]);
        assert_eq!(r.values(&[0xb0, 12, 127]), []); // repeated CC press
        assert_eq!(r.values(&[0xb0, 12, 0]), [ControlValue::Press(false)]);
        // Switches that send 1 for on (Akai AMX crossfader reverse).
        assert_eq!(r.values(&[0xb0, 12, 1]), [ControlValue::Press(true)]);
        assert_eq!(r.values(&[0xb0, 12, 0]), [ControlValue::Press(false)]);
    }

    #[test]
    fn latched_modifier_toggles_per_press() {
        let mut touch = InputBinding::new(InputTarget::Modifier("touch".into()), note(1, 25));
        touch.latch = true;
        let mut kill = InputBinding::new(deck_a(Control::EqHiKill), note(1, 17));
        kill.condition = Some(Condition { modifier: "touch".into(), value: true });
        let mut r = Rig::new(vec![touch, kill]);
        let press = |r: &mut Rig, n: u8| {
            let mut v = r.values(&[0x90, n, 127]);
            v.extend(r.values(&[0x80, n, 0]));
            v
        };
        assert_eq!(press(&mut r, 17), []); // touch mode off
        assert_eq!(press(&mut r, 25), []);
        assert!(r.engine.modifier("touch"), "stays on after release");
        assert_eq!(press(&mut r, 17), [ControlValue::Press(true), ControlValue::Press(false)]);
        press(&mut r, 25);
        assert!(!r.engine.modifier("touch"));
        assert_eq!(press(&mut r, 17), []);
    }

    #[test]
    fn held_modifier_outranks_latched_mode() {
        let mut hotcue = InputBinding::new(InputTarget::Modifier("hotcue".into()), note(1, 39));
        hotcue.latch = true;
        let shift = InputBinding::new(InputTarget::Modifier("shift".into()), note(1, 36));
        let play = InputBinding::new(deck_a(Control::Play), note(1, 0));
        let mut cue = InputBinding::new(deck_a(Control::Hotcue(4)), note(1, 0));
        cue.condition = Some(Condition { modifier: "hotcue".into(), value: true });
        let mut delete = InputBinding::new(deck_a(Control::HotcueDelete(4)), note(1, 0));
        delete.condition = Some(Condition { modifier: "shift".into(), value: true });
        let mut r = Rig::new(vec![hotcue, shift, play, cue, delete]);
        let targets = |r: &mut Rig, msgs: &[[u8; 3]]| {
            let out: Vec<_> = msgs.iter().flat_map(|m| r.send(m)).collect();
            out.iter().filter(|e| e.value == ControlValue::Press(true)).map(|e| e.target).collect::<Vec<_>>()
        };
        let tap = [[0x90, 0, 127], [0x80, 0, 0]];
        assert_eq!(targets(&mut r, &tap), [deck_a(Control::Play)]);
        targets(&mut r, &[[0x90, 39, 127], [0x80, 39, 0]]);
        assert_eq!(targets(&mut r, &tap), [deck_a(Control::Hotcue(4))]);
        let shifted = [[0x90, 36, 127], [0x90, 0, 127], [0x80, 0, 0], [0x80, 36, 0]];
        assert_eq!(targets(&mut r, &shifted), [deck_a(Control::HotcueDelete(4))]);
    }

    #[test]
    fn absolute_with_range_and_invert() {
        let mut b = InputBinding::new(deck_a(Control::Volume), cc(1, 7));
        b.soft_takeover = Some(false);
        b.invert = true;
        b.min = 0.2;
        b.max = 0.6;
        let mut r = Rig::new(vec![b]);
        assert_close(r.values(&[0xb0, 7, 0]), 0.6);
        assert_close(r.values(&[0xb0, 7, 127]), 0.2);
    }

    #[test]
    fn soft_takeover_picks_up_on_cross_or_near() {
        let vol = deck_a(Control::Volume);
        let mut r = Rig::new(vec![InputBinding::new(vol, cc(1, 7))]);
        r.values.set(vol, 0.5);
        // Far below the software value: ignored.
        assert!(r.send(&[0xb0, 7, 10]).is_empty());
        assert!(r.send(&[0xb0, 7, 40]).is_empty());
        // Crossing 0.5 picks up.
        let v = r.values(&[0xb0, 7, 80]);
        assert_eq!(v, [ControlValue::Absolute(80.0 / 127.0)]);
        assert_eq!(r.values(&[0xb0, 7, 20]).len(), 1);

        // Software moves the control (e.g. mouse): pickup resets.
        r.values.set(vol, 0.9);
        assert!(r.send(&[0xb0, 7, 30]).is_empty());
        // Within PICKUP_DISTANCE engages without crossing.
        assert_eq!(r.values(&[0xb0, 7, 112]), [ControlValue::Absolute(112.0 / 127.0)]); // 0.018 away
    }

    #[test]
    fn soft_takeover_first_touch_near_value() {
        let vol = deck_a(Control::Volume);
        let mut r = Rig::new(vec![InputBinding::new(vol, cc(1, 7))]);
        r.values.set(vol, 0.5);
        assert_eq!(r.values(&[0xb0, 7, 64]).len(), 1);
    }

    #[test]
    fn soft_takeover_tolerates_lagging_app() {
        let vol = deck_a(Control::Volume);
        let mut r = Rig::new(vec![InputBinding::new(vol, cc(1, 7))]);
        r.values.set(vol, 0.5);
        let mut out = Vec::new();
        // The app never applies our values during a fast move: still engaged.
        for v in [64u8, 70, 80, 90, 100] {
            r.engine.handle(&[0xb0, 7, v], r.t, &r.values, &mut out);
        }
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn cc14_combines_msb_lsb() {
        let vol = deck_a(Control::Volume);
        let mut b = InputBinding::new(vol, MidiSpec::Cc14 { channel: 1, number: 19 });
        b.soft_takeover = Some(false);
        let mut r = Rig::new(vec![b]);
        // Until an LSB has been seen, MSBs are plain 7-bit values.
        assert_eq!(r.values(&[0xb0, 19, 64]), [ControlValue::Absolute(64.0 / 127.0)]);
        // LSB completes the pair.
        assert_eq!(r.values(&[0xb0, 51, 5]), [ControlValue::Absolute(8197.0 / 16383.0)]);
        // From now on the MSB waits for its LSB.
        assert!(r.send(&[0xb0, 19, 127]).is_empty());
        assert_eq!(r.values(&[0xb0, 51, 127]), [ControlValue::Absolute(1.0)]);
        // A lone MSB is flushed after the window, with the last LSB.
        assert!(r.send(&[0xb0, 19, 0]).is_empty());
        let mut out = Vec::new();
        r.engine.flush(r.t + CC14_WINDOW / 2, &r.values, &mut out);
        assert!(out.is_empty());
        r.engine.flush(r.t + CC14_WINDOW * 2, &r.values, &mut out);
        assert_eq!(out[0].value, ControlValue::Absolute(127.0 / 16383.0));
    }

    #[test]
    fn cc14_lsb_first() {
        let mut b = InputBinding::new(deck_a(Control::Tempo), MidiSpec::Cc14 { channel: 2, number: 0 });
        b.soft_takeover = Some(false);
        let mut r = Rig::new(vec![b]);
        assert_eq!(r.values(&[0xb1, 32, 1]).len(), 1); // first LSB: MSB 0 known
        assert!(r.send(&[0xb1, 32, 0]).is_empty());
        assert_eq!(r.values(&[0xb1, 0, 64]), [ControlValue::Absolute(8192.0 / 16383.0)]);
    }

    #[test]
    fn relative_encodings() {
        let scroll = ControlTarget::global(Control::BrowserScroll);
        for (enc, plus, minus) in
            [(Encoding::TwosComplement, 1, 126), (Encoding::BinaryOffset, 65, 62), (Encoding::SignMagnitude, 1, 66)]
        {
            let mut b = InputBinding::new(scroll, cc(1, 20));
            b.encoding = Some(enc);
            let mut r = Rig::new(vec![b]);
            assert_eq!(r.values(&[0xb0, 20, plus]), [ControlValue::Delta(1.0)], "{enc:?}");
            assert_eq!(r.values(&[0xb0, 20, minus]), [ControlValue::Delta(-2.0)], "{enc:?}");
        }
    }

    #[test]
    fn relative_note_and_step() {
        let scroll = ControlTarget::global(Control::BrowserScroll);
        let mut up = InputBinding::new(scroll, note(1, 1)).with_mode(InputMode::Relative);
        up.step = Some(-1.0);
        let mut r = Rig::new(vec![up]);
        assert_eq!(r.values(&[0x90, 1, 127]), [ControlValue::Delta(-1.0)]);
        assert_eq!(r.values(&[0x80, 1, 0]), []);
    }

    #[test]
    fn relative_on_continuous_accumulates() {
        let eq = deck_a(Control::EqHi);
        let mut r = Rig::new(vec![InputBinding::new(eq, cc(1, 30)).with_mode(InputMode::Relative)]);
        r.values.set(eq, 0.5);
        let lagging = r.values.clone();
        assert_close(r.values(&[0xb0, 30, 5]), 0.55);
        // The app still reports the old value: continue from what we sent.
        let mut out = Vec::new();
        r.engine.handle(&[0xb0, 30, 5], r.t, &lagging, &mut out);
        assert_close(out.iter().map(|e| e.value).collect(), 0.6);
        // Changed elsewhere: continue from there, clamped.
        r.values.set(eq, 0.99);
        assert_close(r.values(&[0xb0, 30, 5]), 1.0);
    }

    #[test]
    fn jog_in_revolutions() {
        let jog = deck_a(Control::Jog);
        let mut b = InputBinding::new(jog, cc(1, 34));
        b.encoding = Some(Encoding::BinaryOffset);
        b.ticks_per_rev = Some(720.0);
        let mut r = Rig::new(vec![b]);
        assert_eq!(r.values(&[0xb0, 34, 64 + 36]), [ControlValue::Delta(0.05)]);
        assert_eq!(r.values(&[0xb0, 34, 64 - 18]), [ControlValue::Delta(-0.025)]);
        assert_eq!(r.values(&[0xb0, 34, 64]), []);
    }

    #[test]
    fn shift_modifier_layers() {
        let shift = Condition { modifier: "shift".into(), value: true };
        let mut shifted = InputBinding::new(deck_a(Control::Reverse), note(1, 11));
        shifted.condition = Some(shift.clone());
        let mut shifted_vol = InputBinding::new(deck_a(Control::Gain), cc(1, 7));
        shifted_vol.condition = Some(shift);
        shifted_vol.soft_takeover = Some(false);
        let mut r = Rig::new(vec![
            InputBinding::new(InputTarget::Modifier("shift".into()), note(1, 63)),
            InputBinding::new(deck_a(Control::Play), note(1, 11)),
            shifted,
            shifted_vol,
        ]);
        let targets = |ev: Vec<ControlEvent>| ev.into_iter().map(|e| e.target.control).collect::<Vec<_>>();
        assert_eq!(targets(r.send(&[0x90, 11, 127])), [Control::Play]);
        assert!(r.send(&[0x90, 63, 127]).is_empty());
        assert!(r.engine.modifier("shift"));
        // Release of the unshifted press still reaches play.
        assert_eq!(targets(r.send(&[0x80, 11, 0])), [Control::Play]);
        assert_eq!(targets(r.send(&[0x90, 11, 127])), [Control::Reverse]);
        assert!(r.send(&[0xb0, 7, 3]).iter().all(|e| e.target.control == Control::Gain));
        // Shift released before the button: release still goes to reverse.
        r.send(&[0x80, 63, 0]);
        assert!(!r.engine.modifier("shift"));
        assert_eq!(targets(r.send(&[0x80, 11, 0])), [Control::Reverse]);
        assert!(r.send(&[0xb0, 7, 3]).is_empty());
    }

    #[test]
    fn next_deck_layout_is_requested_once_per_press() {
        let shift = Condition { modifier: "shift".into(), value: true };
        let mut next = InputBinding::new("deck_layout:next".parse::<InputTarget>().unwrap(), note(1, 33));
        next.condition = Some(shift);
        let mut r = Rig::new(vec![
            InputBinding::new(InputTarget::Modifier("shift".into()), note(1, 10)),
            InputBinding::new(deck_a(Control::Play), note(1, 33)),
            next,
        ]);
        assert_eq!(r.send(&[0x90, 33, 127]).len(), 1);
        r.send(&[0x80, 33, 0]);
        assert!(!r.engine.take_next_deck_layout());
        r.send(&[0x90, 10, 127]);
        assert!(r.send(&[0x90, 33, 127]).is_empty(), "no control event");
        r.send(&[0x80, 33, 0]);
        assert!(r.engine.take_next_deck_layout());
        assert!(!r.engine.take_next_deck_layout(), "taken");
        // The next layout's engine keeps SHIFT held.
        let mut next = MappingEngine::new(r.engine.mapping().clone());
        next.keep_modifiers(&r.engine);
        assert!(next.modifier("shift"));
    }
}
