//! Small typed identifiers.

use serde::{Deserialize, Serialize};

pub const MAX_DECKS: usize = 4;
pub const MAX_FX_UNITS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeckId(pub u8);

impl DeckId {
    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    /// Deck letter as shown in the UI: A, B, C, D.
    pub fn letter(self) -> char {
        char::from(b'A' + self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FxUnitId(pub u8);

impl FxUnitId {
    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}
