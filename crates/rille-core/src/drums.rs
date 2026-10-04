//! Drum machine layout and patterns, shared by engine, controllers and UI.
//!
//! The drum machine has [`INSTRUMENTS`] instrument tracks of [`STEPS`]
//! sixteenth-note steps, and [`PATTERNS`] patterns. A pattern stores its steps
//! as bit masks (bit `s` = step `s + 1`), so it is small and `Copy` and can
//! travel in the engine snapshot.

/// Instrument tracks per pattern.
pub const INSTRUMENTS: usize = 8;
/// Steps per pattern (sixteenths, one bar of 4/4).
pub const STEPS: usize = 16;
pub const PATTERNS: usize = 16;
/// Cells addressed by `drum.cell.N`: `inst * STEPS + step`.
pub const CELLS: usize = INSTRUMENTS * STEPS;

/// Short instrument names, also the keys of a kit file.
pub const NAMES: [&str; INSTRUMENTS] = ["BD", "SD", "CH", "OH", "CP", "RS", "LT", "CY"];

/// Instrument colours, indices into [`crate::remix::COLORS`].
pub const INST_COLORS: [u8; INSTRUMENTS] = [3, 1, 8, 9, 13, 5, 11, 15];

/// Closed hi-hat cuts the open hi-hat short (as on a 909).
pub const CHOKE: (usize, usize) = (2, 3);

/// Gain of a step without accent (accented steps play at 1.0).
pub const NORMAL_GAIN: f32 = 0.7;

/// Note repeat rates in beats: 1/4, 1/8, 1/8 triplet, 1/16, 1/16 triplet,
/// 1/32.
pub const REPEAT_RATES: [f64; 6] = [1.0, 0.5, 1.0 / 3.0, 0.25, 1.0 / 6.0, 0.125];
/// Names of [`REPEAT_RATES`] for displays.
pub const REPEAT_NAMES: [&str; 6] = ["1/4", "1/8", "1/8T", "1/16", "1/16T", "1/32"];
/// Rate a fresh machine rolls at (1/16).
pub const DEFAULT_REPEAT: usize = 3;
/// Velocity from which a hit plays and records as an accent.
pub const ACCENT_VELOCITY: f32 = 0.75;
/// Kits a controller can pick by pad (`drum.kit.N`).
pub const KIT_PADS: usize = 16;

/// Swing at knob 1.0: off-beat sixteenths are delayed by this many beats
/// (half a sixteenth, i.e. 75 % swing).
pub const MAX_SWING_BEATS: f64 = 0.125;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pattern {
    /// Per instrument, bit `s` = step `s + 1` on.
    pub steps: [u16; INSTRUMENTS],
    /// Accents; only meaningful where the step is on.
    pub accents: [u16; INSTRUMENTS],
    /// Steps before the pattern repeats, `1..=16`.
    pub length: u8,
    /// `0..=1`: none … 75 % swing.
    pub swing: f32,
}

impl Default for Pattern {
    fn default() -> Self {
        Self { steps: [0; INSTRUMENTS], accents: [0; INSTRUMENTS], length: STEPS as u8, swing: 0.0 }
    }
}

impl Pattern {
    pub fn is_on(&self, inst: usize, step: usize) -> bool {
        inst < INSTRUMENTS && step < STEPS && self.steps[inst] & (1 << step) != 0
    }

    pub fn is_accent(&self, inst: usize, step: usize) -> bool {
        self.is_on(inst, step) && self.accents[inst] & (1 << step) != 0
    }

    pub fn set(&mut self, inst: usize, step: usize, on: bool) {
        if inst < INSTRUMENTS && step < STEPS {
            if on {
                self.steps[inst] |= 1 << step;
            } else {
                self.steps[inst] &= !(1 << step);
                self.accents[inst] &= !(1 << step);
            }
        }
    }

    /// Sets the accent; an accent on an empty step switches the step on.
    pub fn set_accent(&mut self, inst: usize, step: usize, on: bool) {
        if inst < INSTRUMENTS && step < STEPS {
            if on {
                self.steps[inst] |= 1 << step;
                self.accents[inst] |= 1 << step;
            } else {
                self.accents[inst] &= !(1 << step);
            }
        }
    }

    pub fn has_steps(&self, inst: usize) -> bool {
        inst < INSTRUMENTS && self.steps[inst] != 0
    }

    pub fn clear_row(&mut self, inst: usize) {
        if inst < INSTRUMENTS {
            self.steps[inst] = 0;
            self.accents[inst] = 0;
        }
    }

    /// Moves instrument `inst`'s steps (and accents) by `by` steps, wrapping
    /// round the pattern length; steps past the length stay put.
    pub fn rotate(&mut self, inst: usize, by: i64) {
        if inst >= INSTRUMENTS {
            return;
        }
        let len = self.length();
        let rot = |bits: u16| {
            let mut out = bits & !((1u32 << len) as u16).wrapping_sub(1);
            for s in 0..len {
                if bits & (1 << s) != 0 {
                    out |= 1 << (s as i64 + by).rem_euclid(len as i64);
                }
            }
            out
        };
        self.steps[inst] = rot(self.steps[inst]);
        self.accents[inst] = rot(self.accents[inst]);
    }

    pub fn is_empty(&self) -> bool {
        self.steps.iter().all(|&s| s == 0)
    }

    pub fn length(&self) -> usize {
        usize::from(self.length.clamp(1, STEPS as u8))
    }

    /// One row as text: `.` off, `x` on, `X` accented, one character per step.
    pub fn row_text(&self, inst: usize) -> String {
        (0..STEPS)
            .map(|s| match (self.is_on(inst, s), self.is_accent(inst, s)) {
                (true, true) => 'X',
                (true, false) => 'x',
                _ => '.',
            })
            .collect()
    }

    /// Reads a row written by [`Self::row_text`]; other characters are off,
    /// missing steps stay off.
    pub fn set_row_text(&mut self, inst: usize, text: &str) {
        self.clear_row(inst);
        for (s, c) in text.chars().filter(|c| !c.is_whitespace()).take(STEPS).enumerate() {
            match c {
                'x' | 'o' | '1' => self.set(inst, s, true),
                'X' | 'O' => self.set_accent(inst, s, true),
                _ => {}
            }
        }
    }

    fn from_rows(rows: &[(usize, &str)]) -> Self {
        let mut p = Self::default();
        for &(inst, text) in rows {
            p.set_row_text(inst, text);
        }
        p
    }
}

/// When step `s` (an absolute sixteenth count from the bar origin) plays, in
/// beats from the origin. Odd steps (the off-beat sixteenths) are delayed by
/// the swing.
pub fn step_time(s: i64, swing: f32) -> f64 {
    let base = s as f64 * 0.25;
    if s.rem_euclid(2) == 1 { base + f64::from(swing.clamp(0.0, 1.0)) * MAX_SWING_BEATS } else { base }
}

/// Step within a pattern of `length` steps for absolute step `s`.
pub fn pattern_step(s: i64, length: usize) -> usize {
    s.rem_euclid(length.clamp(1, STEPS) as i64) as usize
}

/// Patterns a fresh installation starts with: a few grooves to play along
/// with, the rest empty.
pub fn factory_patterns() -> [Pattern; PATTERNS] {
    const BD: usize = 0;
    const SD: usize = 1;
    const CH: usize = 2;
    const OH: usize = 3;
    const CP: usize = 4;
    const RS: usize = 5;
    const LT: usize = 6;
    const CY: usize = 7;
    let mut p = [Pattern::default(); PATTERNS];
    // House: four to the floor, clap on 2 and 4, open hat on the off-beats.
    p[0] = Pattern::from_rows(&[
        (BD, "X...x...X...x..."),
        (CP, "....x.......x..."),
        (CH, "x.x.x.x.x.x.x.x."),
        (OH, "..x...x...x...x."),
    ]);
    // Techno: driving sixteenth hats, rim shots, a crash on the one.
    p[1] = Pattern::from_rows(&[
        (BD, "X...x...X...x..."),
        (CH, "xxXxxxXxxxXxxxXx"),
        (OH, "..x...x...x...x."),
        (RS, "...x..x....x..x."),
        (CY, "X..............."),
    ]);
    // 2-step garage.
    p[2] = Pattern::from_rows(&[
        (BD, "X.........x....."),
        (SD, "....X.......X..."),
        (CH, "x.xxx.x.x.xxx.x."),
        (RS, "..........x...x."),
    ]);
    // Breakbeat.
    p[3] = Pattern::from_rows(&[
        (BD, "X.........X.x..."),
        (SD, "....X..x.x..X..x"),
        (CH, "x.x.x.x.x.x.x.x."),
        (OH, "..............x."),
    ]);
    // Minimal.
    p[4] = Pattern::from_rows(&[
        (BD, "X...x...x...x..."),
        (RS, "..x.....x.x....."),
        (CH, "..x...x...x...x."),
        (LT, ".......x......x."),
    ]);
    // Rolling hats over a straight kick.
    p[5] = Pattern::from_rows(&[(BD, "X...x...X...x..."), (CH, "xXxxxXxxxXxxxXxx"), (CP, "....x.......x...")]);
    // Claps and hats only, to ride on top of a track.
    p[6] = Pattern::from_rows(&[(CP, "....X.......X..."), (CH, "..x...x...x...x.")]);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_and_accents() {
        let mut p = Pattern::default();
        assert_eq!(p.length(), 16);
        p.set(0, 3, true);
        assert!(p.is_on(0, 3) && !p.is_accent(0, 3));
        p.set_accent(1, 5, true);
        assert!(p.is_on(1, 5) && p.is_accent(1, 5));
        p.set(1, 5, false);
        assert!(!p.is_on(1, 5) && p.accents[1] == 0, "switching off clears the accent");
        assert!(!p.is_on(9, 0) && !p.is_on(0, 16));
        p.set(9, 0, true);
        p.set(0, 16, true);
        assert_eq!(p.steps[0], 1 << 3);
    }

    #[test]
    fn row_text_roundtrip() {
        for pat in factory_patterns() {
            for inst in 0..INSTRUMENTS {
                let mut q = Pattern::default();
                q.set_row_text(inst, &pat.row_text(inst));
                assert_eq!(q.steps[inst], pat.steps[inst]);
                assert_eq!(q.accents[inst], pat.accents[inst]);
            }
        }
        let mut p = Pattern::default();
        p.set_row_text(0, "x... X");
        assert!(p.is_on(0, 0) && p.is_accent(0, 4) && !p.is_on(0, 5));
    }

    #[test]
    fn rotate_wraps_round_the_length() {
        let mut p = Pattern::default();
        p.set_row_text(0, "X..x............");
        p.rotate(0, 1);
        assert_eq!(p.row_text(0), ".X..x...........");
        p.rotate(0, -2);
        assert_eq!(p.row_text(0), "..x............X");
        p.length = 4;
        p.set_row_text(1, "x..x....x.......");
        p.rotate(1, 1);
        assert_eq!(p.row_text(1), "xx......x.......", "step 9 is past the length");
    }

    #[test]
    fn swing_delays_odd_steps() {
        assert_eq!(step_time(0, 1.0), 0.0);
        assert_eq!(step_time(1, 0.0), 0.25);
        assert_eq!(step_time(1, 1.0), 0.25 + MAX_SWING_BEATS);
        assert_eq!(step_time(4, 1.0), 1.0);
        assert_eq!(step_time(-1, 1.0), -0.25 + MAX_SWING_BEATS);
        assert_eq!(pattern_step(17, 16), 1);
        assert_eq!(pattern_step(-1, 16), 15);
        assert_eq!(pattern_step(7, 3), 1);
    }

    #[test]
    fn factory_patterns_have_grooves() {
        let p = factory_patterns();
        assert!(p[0].has_steps(0) && !p[0].is_empty());
        assert!(p[PATTERNS - 1].is_empty());
    }
}
