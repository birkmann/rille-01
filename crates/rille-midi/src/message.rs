//! Raw MIDI short messages.
//!
//! Channels are `0..=15` here; mapping files use `1..=16`.

/// One parsed channel voice message. Everything the mapping engine does not
/// use (aftertouch, program change, clock, SysEx, …) is [`MidiMsg::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMsg {
    NoteOn {
        ch: u8,
        note: u8,
        vel: u8,
    },
    /// Also produced for a note-on with velocity 0.
    NoteOff {
        ch: u8,
        note: u8,
        vel: u8,
    },
    Cc {
        ch: u8,
        num: u8,
        val: u8,
    },
    /// 14-bit value, `8192` = center.
    PitchBend {
        ch: u8,
        val14: u16,
    },
    Other,
}

impl MidiMsg {
    pub fn parse(raw: &[u8]) -> MidiMsg {
        let (&status, data) = match raw.split_first() {
            Some(x) => x,
            None => return MidiMsg::Other,
        };
        let ch = status & 0x0f;
        let d = |i: usize| data.get(i).copied().filter(|b| b & 0x80 == 0);
        let (Some(a), Some(b)) = (d(0), d(1)) else { return MidiMsg::Other };
        match status & 0xf0 {
            0x90 if b > 0 => MidiMsg::NoteOn { ch, note: a, vel: b },
            0x80 | 0x90 => MidiMsg::NoteOff { ch, note: a, vel: b },
            0xb0 => MidiMsg::Cc { ch, num: a, val: b },
            0xe0 => MidiMsg::PitchBend { ch, val14: u16::from(a) | u16::from(b) << 7 },
            _ => MidiMsg::Other,
        }
    }

    pub fn channel(self) -> Option<u8> {
        match self {
            MidiMsg::NoteOn { ch, .. }
            | MidiMsg::NoteOff { ch, .. }
            | MidiMsg::Cc { ch, .. }
            | MidiMsg::PitchBend { ch, .. } => Some(ch),
            MidiMsg::Other => None,
        }
    }

    /// Encodes the message; `None` for [`MidiMsg::Other`].
    pub fn to_bytes(self) -> Option<[u8; 3]> {
        Some(match self {
            MidiMsg::NoteOn { ch, note, vel } => [0x90 | ch, note, vel],
            MidiMsg::NoteOff { ch, note, vel } => [0x80 | ch, note, vel],
            MidiMsg::Cc { ch, num, val } => [0xb0 | ch, num, val],
            MidiMsg::PitchBend { ch, val14 } => [0xe0 | ch, (val14 & 0x7f) as u8, (val14 >> 7 & 0x7f) as u8],
            MidiMsg::Other => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_voice_messages() {
        assert_eq!(MidiMsg::parse(&[0x91, 11, 127]), MidiMsg::NoteOn { ch: 1, note: 11, vel: 127 });
        assert_eq!(MidiMsg::parse(&[0x91, 11, 0]), MidiMsg::NoteOff { ch: 1, note: 11, vel: 0 });
        assert_eq!(MidiMsg::parse(&[0x8f, 60, 64]), MidiMsg::NoteOff { ch: 15, note: 60, vel: 64 });
        assert_eq!(MidiMsg::parse(&[0xb0, 7, 100]), MidiMsg::Cc { ch: 0, num: 7, val: 100 });
        assert_eq!(MidiMsg::parse(&[0xe2, 0, 64]), MidiMsg::PitchBend { ch: 2, val14: 8192 });
        assert_eq!(MidiMsg::parse(&[0xe2, 0x7f, 0x7f]), MidiMsg::PitchBend { ch: 2, val14: 16383 });
    }

    #[test]
    fn rejects_other_and_malformed() {
        assert_eq!(MidiMsg::parse(&[]), MidiMsg::Other);
        assert_eq!(MidiMsg::parse(&[0xf8]), MidiMsg::Other); // clock
        assert_eq!(MidiMsg::parse(&[0xc0, 5]), MidiMsg::Other); // program change
        assert_eq!(MidiMsg::parse(&[0x90, 60]), MidiMsg::Other); // truncated
        assert_eq!(MidiMsg::parse(&[0xb0, 0x80, 1]), MidiMsg::Other); // data byte with high bit
        assert_eq!(MidiMsg::parse(&[0xf0, 1, 2, 0xf7]), MidiMsg::Other);
    }

    #[test]
    fn bytes_roundtrip() {
        for raw in [[0x90, 1, 2], [0x8a, 3, 0], [0xbf, 127, 64], [0xe0, 0x12, 0x34]] {
            assert_eq!(MidiMsg::parse(&raw).to_bytes(), Some(raw));
        }
    }
}
