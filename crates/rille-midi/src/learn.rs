//! MIDI learn: watch what a control sends and infer a binding for it.

use crate::mapping::{Encoding, InputBinding, InputMode, MidiSpec};
use crate::message::MidiMsg;
use rille_core::{ControlKind, ControlTarget};
use std::collections::HashMap;
use std::time::Instant;

/// Messages kept per session; enough for a few knob sweeps.
const MAX_MESSAGES: usize = 512;

/// Collects messages while the user moves one hardware control, then guesses
/// the binding for `target`.
#[derive(Clone, Debug)]
pub struct LearnSession {
    target: ControlTarget,
    msgs: Vec<(MidiMsg, Instant)>,
}

impl LearnSession {
    pub fn new(target: ControlTarget) -> Self {
        Self { target, msgs: Vec::new() }
    }

    pub fn target(&self) -> ControlTarget {
        self.target
    }

    pub fn feed(&mut self, raw: &[u8], t: Instant) {
        let msg = MidiMsg::parse(raw);
        if msg != MidiMsg::Other && self.msgs.len() < MAX_MESSAGES {
            self.msgs.push((msg, t));
        }
    }

    pub fn clear(&mut self) {
        self.msgs.clear();
    }

    /// The inferred binding, or `None` while the evidence is insufficient:
    ///
    /// - note → button (a relative control gets one step per press)
    /// - CC on a button control → button (value > 0 = pressed)
    /// - CC values clustered at 1/127, 63/65 or 1/65 → relative with that encoding
    /// - CC sweep → absolute; CC n together with n+32 → 14-bit
    /// - pitch bend → absolute (relative for relative controls)
    pub fn result(&self) -> Option<InputBinding> {
        let kind = self.target.control.kind();
        let bind = |midi| Some(InputBinding::new(self.target, midi));
        let first_note = self.msgs.iter().find_map(|(m, _)| match *m {
            MidiMsg::NoteOn { ch, note, .. } => Some(MidiSpec::Note { channel: ch + 1, number: note }),
            _ => None,
        });
        if let (ControlKind::Button, Some(note)) = (kind, first_note) {
            return bind(note);
        }

        // Most frequent CC; an LSB (n ≥ 32) counts towards its MSB.
        let mut counts: HashMap<(u8, u8), usize> = HashMap::new();
        for (m, _) in &self.msgs {
            if let MidiMsg::Cc { ch, num, .. } = *m {
                *counts.entry((ch, num)).or_default() += 1;
            }
        }
        let best = counts.iter().max_by_key(|(k, n)| (**n, std::cmp::Reverse(**k))).map(|(k, _)| *k);
        if let Some((ch, mut num)) = best {
            if num >= 32 && counts.contains_key(&(ch, num - 32)) {
                num -= 32;
            }
            let channel = ch + 1;
            let vals: Vec<u8> = self
                .msgs
                .iter()
                .filter_map(|(m, _)| match *m {
                    MidiMsg::Cc { ch: c, num: n, val } if (c, n) == (ch, num) => Some(val),
                    _ => None,
                })
                .collect();
            if kind == ControlKind::Button {
                return bind(MidiSpec::Cc { channel, number: num });
            }
            if let Some(enc) = relative_encoding(&vals) {
                let mut b = InputBinding::new(self.target, MidiSpec::Cc { channel, number: num });
                b.encoding = Some(enc);
                if kind == ControlKind::Continuous {
                    b.mode = Some(InputMode::Relative);
                }
                return Some(b);
            }
            let lsbs = if num < 32 { counts.get(&(ch, num + 32)).copied().unwrap_or(0) } else { 0 };
            if kind == ControlKind::Continuous && distinct(&vals) >= 3 {
                let wide = lsbs * 2 >= vals.len();
                return bind(if wide {
                    MidiSpec::Cc14 { channel, number: num }
                } else {
                    MidiSpec::Cc { channel, number: num }
                });
            }
            return None;
        }

        if let Some(note) = first_note {
            return (kind == ControlKind::Relative)
                .then(|| InputBinding::new(self.target, note).with_mode(InputMode::Relative));
        }
        let bend = self.msgs.iter().find_map(|(m, _)| match *m {
            MidiMsg::PitchBend { ch, .. } => Some(MidiSpec::PitchBend { channel: ch + 1 }),
            _ => None,
        });
        bend.filter(|_| kind != ControlKind::Button).and_then(bind)
    }
}

fn distinct(vals: &[u8]) -> usize {
    let mut seen = [false; 128];
    vals.iter().filter(|&&v| !std::mem::replace(&mut seen[usize::from(v & 0x7f)], true)).count()
}

/// Encoders repeat values around their zero point (1, 1, 127, …); a knob
/// sweep never sends the same value twice in a row and never jumps.
fn relative_encoding(vals: &[u8]) -> Option<Encoding> {
    let encoder_like = vals.windows(2).any(|w| w[0] == w[1] || w[0].abs_diff(w[1]) > 40);
    if !encoder_like {
        return None;
    }
    let all = |f: fn(u8) -> bool| vals.iter().all(|&v| f(v));
    let any = |f: fn(u8) -> bool| vals.iter().any(|&v| f(v));
    let small = |v| (1..=15).contains(&v);
    let twos = all(|v| (1..=15).contains(&v) || (113..=127).contains(&v));
    let sign_mag = all(|v| (1..=15).contains(&v) || (65..=79).contains(&v));
    let offset = all(|v| (49..=79).contains(&v));
    if twos && (any(|v| v >= 113) || !sign_mag) {
        Some(Encoding::TwosComplement)
    } else if sign_mag && any(small) && any(|v| v >= 65) {
        Some(Encoding::SignMagnitude)
    } else if twos {
        Some(Encoding::TwosComplement)
    } else if offset {
        Some(Encoding::BinaryOffset)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::InputTarget;
    use rille_core::Control;

    fn learn(control: Control, msgs: &[[u8; 3]]) -> Option<InputBinding> {
        let mut s = LearnSession::new(ControlTarget::deck(0, control));
        let t = Instant::now();
        msgs.iter().for_each(|m| s.feed(m, t));
        s.result()
    }

    fn cc_seq(ch: u8, num: u8, vals: impl IntoIterator<Item = u8>) -> Vec<[u8; 3]> {
        vals.into_iter().map(|v| [0xb0 | ch, num, v]).collect()
    }

    #[test]
    fn note_becomes_button() {
        let b = learn(Control::Play, &[[0x91, 11, 127], [0x81, 11, 0]]).unwrap();
        assert_eq!(b.midi, MidiSpec::Note { channel: 2, number: 11 });
        assert_eq!(b.mode(), InputMode::Button);
        assert_eq!(b.target, InputTarget::Control(ControlTarget::deck(0, Control::Play)));
        assert!(b.validate().is_ok());
    }

    #[test]
    fn cc_on_button_control_becomes_button() {
        let b = learn(Control::Cue, &[[0xb0, 20, 127], [0xb0, 20, 0]]).unwrap();
        assert_eq!(b.midi, MidiSpec::Cc { channel: 1, number: 20 });
        assert_eq!(b.mode(), InputMode::Button);
    }

    #[test]
    fn sweep_becomes_absolute() {
        let b = learn(Control::Volume, &cc_seq(0, 7, 20..60)).unwrap();
        assert_eq!(b.midi, MidiSpec::Cc { channel: 1, number: 7 });
        assert_eq!(b.mode(), InputMode::Absolute);
        assert!(learn(Control::Volume, &cc_seq(0, 7, [5])).is_none()); // not enough yet
    }

    #[test]
    fn msb_lsb_pairs_become_cc14() {
        let msgs: Vec<_> = (0..20u8).flat_map(|i| [[0xb1, 0x13, 40 + i], [0xb1, 0x33, i * 5]]).collect();
        let b = learn(Control::Volume, &msgs).unwrap();
        assert_eq!(b.midi, MidiSpec::Cc14 { channel: 2, number: 0x13 });
        assert!(b.validate().is_ok());
    }

    #[test]
    fn relative_encodings_detected() {
        let cases = [
            (vec![1, 1, 2, 1, 127, 127, 126], Encoding::TwosComplement),
            (vec![1, 1, 1, 1], Encoding::TwosComplement),
            (vec![65, 65, 66, 63, 63, 62], Encoding::BinaryOffset),
            (vec![65, 65, 65], Encoding::BinaryOffset),
            (vec![1, 1, 2, 65, 65, 66], Encoding::SignMagnitude),
        ];
        for (vals, enc) in cases {
            let b = learn(Control::Jog, &cc_seq(0, 34, vals.clone())).unwrap();
            assert_eq!(b.encoding, Some(enc), "{vals:?}");
            assert_eq!(b.mode(), InputMode::Jog);
            // Same on a continuous control: relative mode stepping the value.
            let b = learn(Control::EqHi, &cc_seq(0, 34, vals.clone())).unwrap();
            assert_eq!((b.mode(), b.encoding), (InputMode::Relative, Some(enc)), "{vals:?}");
        }
        assert!(learn(Control::Jog, &cc_seq(0, 34, 10..40)).is_none());
    }

    #[test]
    fn pitchbend_becomes_absolute() {
        let b = learn(Control::Tempo, &[[0xe0, 0, 60], [0xe0, 0, 62], [0xe0, 0, 64]]).unwrap();
        assert_eq!(b.midi, MidiSpec::PitchBend { channel: 1 });
        assert_eq!(b.mode(), InputMode::Absolute);
    }

    #[test]
    fn jog_touch_note_with_rotation_prefers_cc_for_jog() {
        let mut msgs = vec![[0x90, 54, 127]];
        msgs.extend(cc_seq(0, 34, [65, 65, 66, 65]));
        let b = learn(Control::Jog, &msgs).unwrap();
        assert_eq!(b.midi, MidiSpec::Cc { channel: 1, number: 34 });
        let b = learn(Control::JogTouch, &msgs).unwrap();
        assert_eq!(b.midi, MidiSpec::Note { channel: 1, number: 54 });
    }
}
