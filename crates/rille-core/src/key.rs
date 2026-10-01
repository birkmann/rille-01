//! Musical key with Camelot / Open Key notation.

use serde::{Deserialize, Serialize};

const NAMES: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

/// 0..=11 major keys C..B, 12..=23 minor keys Cm..Bm (same numbering as the
/// NML `MUSICAL_KEY` value).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Key(u8);

impl TryFrom<u8> for Key {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, u8> {
        if v < 24 { Ok(Self(v)) } else { Err(v) }
    }
}

impl From<Key> for u8 {
    fn from(k: Key) -> u8 {
        k.0
    }
}

impl Key {
    pub fn new(pitch_class: u8, minor: bool) -> Self {
        Self(pitch_class % 12 + if minor { 12 } else { 0 })
    }

    pub fn pitch_class(self) -> u8 {
        self.0 % 12
    }

    pub fn is_minor(self) -> bool {
        self.0 >= 12
    }

    /// Camelot wheel number 1..=12 (A = minor, B = major).
    pub fn camelot_number(self) -> u8 {
        // Minor keys share the number of their relative major (3 semitones up).
        let major_pc = if self.is_minor() { (self.pitch_class() + 3) % 12 } else { self.pitch_class() };
        (7 * major_pc + 7) % 12 + 1
    }

    pub fn camelot(self) -> String {
        format!("{}{}", self.camelot_number(), if self.is_minor() { 'A' } else { 'B' })
    }

    pub fn open_key(self) -> String {
        let n = (i16::from(self.camelot_number()) - 8).rem_euclid(12) + 1;
        format!("{}{}", n, if self.is_minor() { 'm' } else { 'd' })
    }

    pub fn musical(self) -> String {
        let name = NAMES[usize::from(self.pitch_class())];
        if self.is_minor() { format!("{name}m") } else { name.to_owned() }
    }

    /// Hue on the key color wheel; neighbouring (compatible) keys get
    /// neighbouring hues.
    pub fn hue_degrees(self) -> f32 {
        f32::from(self.camelot_number() - 1) * 30.0
    }

    /// Transpose by semitones, keeping the mode.
    pub fn transposed(self, semitones: i8) -> Self {
        let pc = (i16::from(self.pitch_class()) + i16::from(semitones)).rem_euclid(12) as u8;
        Self::new(pc, self.is_minor())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camelot_and_open_key() {
        let cases = [
            (Key::new(0, false), "8B", "1d", "C"),
            (Key::new(9, true), "8A", "1m", "Am"),
            (Key::new(7, false), "9B", "2d", "G"),
            (Key::new(2, true), "7A", "12m", "Dm"),
            (Key::new(6, true), "11A", "4m", "F#m"),
            (Key::new(11, false), "1B", "6d", "B"),
        ];
        for (k, cam, open, name) in cases {
            assert_eq!(k.camelot(), cam, "{name}");
            assert_eq!(k.open_key(), open, "{name}");
            assert_eq!(k.musical(), name);
        }
    }

    #[test]
    fn camelot_numbers_are_a_bijection() {
        let mut seen = [[false; 2]; 12];
        for v in 0..24u8 {
            let k = Key::try_from(v).unwrap();
            let slot = &mut seen[usize::from(k.camelot_number() - 1)][usize::from(k.is_minor())];
            assert!(!*slot);
            *slot = true;
        }
        assert!(Key::try_from(24).is_err());
    }

    #[test]
    fn transpose() {
        assert_eq!(Key::new(11, true).transposed(1), Key::new(0, true));
        assert_eq!(Key::new(0, false).transposed(-1), Key::new(11, false));
    }
}
