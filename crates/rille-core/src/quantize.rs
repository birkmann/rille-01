//! Beat arithmetic used by quantized jumps, loops and sync.
//!
//! These operate on beat positions from a [`crate::BeatClock`], so they work
//! the same for constant, piecewise and live grids.

/// Phase within the beat, in `[0, 1)`.
pub fn beat_phase(beat: f64) -> f64 {
    beat.rem_euclid(1.0)
}

/// Wraps a phase difference into `[-0.5, 0.5)` beats (shortest way round).
pub fn wrap_phase_error(err: f64) -> f64 {
    (err + 0.5).rem_euclid(1.0) - 0.5
}

/// Wraps a phase difference into `[-period/2, period/2)`, e.g. a bar for bar sync.
pub fn wrap_phase_error_mod(err: f64, period: f64) -> f64 {
    (err + period * 0.5).rem_euclid(period) - period * 0.5
}

/// Phase-preserving jump: the position closest to `target_beat` that keeps
/// the current offset within the beat. Jumping to a cue while in sync keeps
/// the deck in phase.
pub fn phase_preserving_target(now_beat: f64, target_beat: f64) -> f64 {
    now_beat + (target_beat - now_beat).round()
}

/// A loop in beat units. `wrap` maps a position that ran past the end back
/// into the loop with the overshoot carried, so repeated wrapping never
/// accumulates drift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatLoop {
    pub start_beat: f64,
    pub len_beats: f64,
}

impl BeatLoop {
    pub fn end_beat(&self) -> f64 {
        self.start_beat + self.len_beats
    }

    /// Returns `beat` unchanged inside the loop or before it (so a loop set
    /// ahead of the playhead is entered naturally); positions at or past the
    /// end are wrapped.
    pub fn wrap(&self, beat: f64) -> f64 {
        if beat < self.end_beat() {
            return beat;
        }
        let over = (beat - self.start_beat).rem_euclid(self.len_beats);
        self.start_beat + over
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn phase_helpers() {
        assert_abs_diff_eq!(beat_phase(-0.25), 0.75);
        assert_abs_diff_eq!(wrap_phase_error(0.75), -0.25);
        assert_abs_diff_eq!(wrap_phase_error(-0.6), 0.4, epsilon = 1e-12);
        assert_abs_diff_eq!(wrap_phase_error_mod(3.5, 4.0), -0.5);
    }

    #[test]
    fn jump_keeps_phase() {
        let t = phase_preserving_target(10.3, 32.0);
        assert_abs_diff_eq!(t, 32.3, epsilon = 1e-12);
        let t = phase_preserving_target(10.7, 32.0);
        assert_abs_diff_eq!(t, 31.7, epsilon = 1e-12);
    }

    #[test]
    fn loop_wrap_carries_overshoot() {
        let l = BeatLoop { start_beat: 16.0, len_beats: 4.0 };
        assert_abs_diff_eq!(l.wrap(18.0), 18.0);
        assert_abs_diff_eq!(l.wrap(20.25), 16.25);
        assert_abs_diff_eq!(l.wrap(3.0), 3.0);
    }
}
