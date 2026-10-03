//! Current control values from the engine snapshot, for MIDI soft-takeover
//! and controller LEDs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rille_core::drums::{INST_COLORS, INSTRUMENTS, PATTERNS, STEPS};
use rille_core::remix::{CELLS, SLOTS, led_code, pad_cell};
use rille_core::{Control, ControlTarget, Scope};
use rille_engine::{DrumState, MAX_DECKS, REMIX_QUANT_SIZES, RemixState, Snapshot};
use rille_midi::ValueSource;
use rille_midi::hid::DISPLAY_LOOP_SIZE;

use crate::engine_slot::EngineSlot;

/// The engine's state, plus whether the main mix is being recorded and
/// whether the drum machine is shown.
pub struct SnapshotValues {
    engine: Arc<EngineSlot>,
    recording: Arc<AtomicBool>,
    drums_visible: Arc<AtomicBool>,
    /// One snapshot for a whole round of LED updates (taking one copies it).
    frozen: Option<Snapshot>,
}

impl SnapshotValues {
    /// Reads the engine's latest state on every call (soft takeover).
    pub fn live(engine: Arc<EngineSlot>, recording: Arc<AtomicBool>, drums_visible: Arc<AtomicBool>) -> Self {
        Self { engine, recording, drums_visible, frozen: None }
    }

    /// The engine's state now, for one round of controller LEDs.
    pub fn frozen(engine: &Arc<EngineSlot>, recording: Arc<AtomicBool>, drums_visible: Arc<AtomicBool>) -> Self {
        let frozen = engine.get().map(|e| e.snapshot());
        Self { engine: engine.clone(), recording, drums_visible, frozen }
    }

    fn snapshot(&self) -> Option<Snapshot> {
        self.frozen.or_else(|| self.engine.get().map(|e| e.snapshot()))
    }
}

fn b(v: bool) -> f32 {
    if v { 1.0 } else { 0.0 }
}

/// LED code of a remix cell: bright while it plays, blinking with the beat
/// while it waits to start, dim when loaded, off when empty.
fn cell_led(r: &RemixState, cell: usize, beat_phase: f64) -> f32 {
    let Some(c) = r.cells.get(cell).filter(|c| c.loaded) else { return 0.0 };
    let slot = &r.slots[cell / (CELLS / SLOTS)];
    let bright = if slot.queued == Some(cell as u8) { beat_phase < 0.5 } else { slot.cell == Some(cell as u8) };
    f32::from(led_code(c.color, bright))
}

impl ValueSource for SnapshotValues {
    fn value(&self, t: ControlTarget) -> f32 {
        if t.control == Control::Record {
            return b(self.recording.load(Ordering::Relaxed));
        }
        if t.control == Control::DrumShow {
            return b(self.drums_visible.load(Ordering::Relaxed));
        }
        let Some(s) = self.snapshot() else { return t.control.default_value() };
        if t.control.scope() == Scope::Drum {
            return drum_value(t.control, &s.drums, s.clock_beat.rem_euclid(1.0));
        }
        let u = usize::from(t.unit);
        if u >= MAX_DECKS {
            return 0.0;
        }
        let (d, c, r) = (&s.decks[u], &s.channels[u], &s.remix[u]);
        let fx = &s.fx[u.min(s.fx.len() - 1)];
        let slot = |n: u8| r.slots.get(usize::from(n).wrapping_sub(1));
        match t.control {
            Control::Play => b(d.playing),
            Control::Cue | Control::Cup => b(d.cue_held),
            Control::Sync => b(d.sync),
            Control::Master => b(d.master),
            Control::Keylock => b(d.keylock),
            Control::Flux => b(d.flux),
            Control::Reverse => b(d.reverse),
            Control::Tick => b(d.tick),
            Control::Hotcue(n) => b(n >= 1 && d.hotcues.get(usize::from(n - 1)).is_some_and(|h| h.is_some())),
            Control::LoopToggle | Control::LoopIn | Control::LoopOut => b(d.loop_active),
            // The loop size as a segment display code.
            Control::LoopSizeUp | Control::LoopSizeDown => f32::from(DISPLAY_LOOP_SIZE) + d.loop_size_idx as f32,
            Control::Tempo => d.tempo_fader,
            Control::KeyShift => d.key_shift / 24.0 + 0.5,
            Control::Seek => (d.position_secs / d.duration_secs.max(1e-9)) as f32,
            Control::Gain => c.gain,
            Control::EqHi => c.eq[2],
            Control::EqMid => c.eq[1],
            Control::EqLo => c.eq[0],
            Control::EqLoKill => b(c.kill[0]),
            Control::EqMidKill => b(c.kill[1]),
            Control::EqHiKill => b(c.kill[2]),
            Control::Filter => c.filter,
            Control::Volume => c.volume,
            Control::Pfl => b(c.pfl),
            Control::Meter => c.meter[0].max(c.meter[1]),
            Control::FxAssign(n) => b(n >= 1 && c.fx_assign.get(usize::from(n - 1)).copied().unwrap_or(false)),
            Control::FxOn => b(fx.on),
            Control::FxDryWet => fx.dry_wet,
            Control::FxKnob(n) => fx.amount.get(usize::from(n.max(1) - 1)).copied().unwrap_or(0.0),
            Control::FxButton(n) => b(fx.enabled.get(usize::from(n.max(1) - 1)).copied().unwrap_or(false)),
            Control::FxParam(n) => fx.knobs.get(usize::from(n.max(1) - 1)).map_or(0.0, |k| k[0]),
            Control::Crossfader => s.crossfader,
            Control::CrossfaderCurve => s.crossfader_curve,
            Control::CrossfaderReverse => b(s.crossfader_reverse),
            Control::MainLevel => s.main_level,
            Control::MainMeter(n) => s.master_meter[usize::from(n.clamp(1, 2) - 1)],
            Control::CueMix => s.cue_mix,
            Control::CueVolume => s.cue_volume,
            Control::Quantize => b(s.quantize),
            Control::Snap => b(s.snap),
            Control::Limiter => b(s.limiter),
            // Remix decks; LED codes and the page number are plain numbers.
            Control::RemixCell(n) => cell_led(r, usize::from(n).wrapping_sub(1), s.clock_beat.rem_euclid(1.0)),
            Control::RemixPad(n) => {
                pad_cell(n, usize::from(r.page)).map_or(0.0, |cell| cell_led(r, cell, s.clock_beat.rem_euclid(1.0)))
            }
            // Blank on a two-digit display (127) unless it is a remix deck.
            Control::RemixPage => {
                if r.active {
                    f32::from(r.page) + 1.0
                } else {
                    127.0
                }
            }
            Control::StemVolume(n @ 1..=4) => d.stem_volume[usize::from(n - 1)],
            Control::StemMute(n @ 1..=4) => b(d.stem_mute[usize::from(n - 1)]),
            Control::RemixStop(n) => b(slot(n).is_some_and(|s| s.cell.is_some())),
            Control::RemixMute(n) => b(slot(n).is_some_and(|s| s.muted)),
            Control::RemixVolume(n) => slot(n).map_or(1.0, |s| s.volume),
            Control::RemixFilter(n) => slot(n).map_or(0.5, |s| s.filter),
            Control::RemixQuantize => b(r.quantize),
            // The source deck as a letter on a two-digit display (100 = A).
            Control::RemixCaptureSource => 100.0 + f32::from(r.capture_source),
            Control::RemixQuantizeSize => f32::from(r.quant_idx) / (REMIX_QUANT_SIZES.len() - 1) as f32,
            other => other.default_value(),
        }
    }

    fn beat_phase(&self) -> f64 {
        self.snapshot().map_or(0.0, |s| s.clock_beat.rem_euclid(1.0))
    }
}

/// Drum machine controls read back for controller LEDs and soft takeover.
fn drum_value(c: Control, d: &DrumState, beat_phase: f64) -> f32 {
    let idx = |n: u8, max: usize| usize::from(n).checked_sub(1).filter(|&i| i < max);
    let pat = &d.patterns[usize::from(d.current).min(PATTERNS - 1)];
    let sel = usize::from(d.selected).min(INSTRUMENTS - 1);
    let inst = |n: u8| idx(n, INSTRUMENTS).map(|i| &d.inst[i]);
    let playhead = |s: usize| d.step == Some(s as u8);
    match c {
        Control::DrumPlay => b(d.playing),
        Control::DrumRecord => b(d.record),
        // A running light: the playhead inverts the step under it.
        Control::DrumStep(n) => idx(n, STEPS).map_or(0.0, |s| b(pat.is_on(sel, s) != playhead(s))),
        Control::DrumAccent(n) => idx(n, STEPS).map_or(0.0, |s| b(pat.is_accent(sel, s))),
        Control::DrumStepLed(n) => idx(n, STEPS).map_or(0.0, |s| {
            if playhead(s) {
                f32::from(led_code(0, true))
            } else if pat.is_on(sel, s) {
                f32::from(led_code(INST_COLORS[sel], pat.is_accent(sel, s) || !d.playing))
            } else {
                0.0
            }
        }),
        Control::DrumCell(n) => idx(n, STEPS * INSTRUMENTS).map_or(0.0, |c| b(pat.is_on(c / STEPS, c % STEPS))),
        Control::DrumCellAccent(n) => {
            idx(n, STEPS * INSTRUMENTS).map_or(0.0, |c| b(pat.is_accent(c / STEPS, c % STEPS)))
        }
        Control::DrumInst(n) => idx(n, INSTRUMENTS).map_or(0.0, |i| b(i == sel)),
        Control::DrumInstLed(n) => idx(n, INSTRUMENTS).map_or(0.0, |i| {
            if i == sel || pat.has_steps(i) { f32::from(led_code(INST_COLORS[i], i == sel)) } else { 0.0 }
        }),
        Control::DrumTrigger(n) => inst(n).map_or(0.0, |i| b(i.loaded && !i.muted)),
        Control::DrumInstMute(n) => inst(n).map_or(0.0, |i| b(i.muted)),
        Control::DrumInstLevel(n) => inst(n).map_or(1.0, |i| i.level),
        Control::DrumInstTune(n) => inst(n).map_or(0.5, |i| i.tune),
        Control::DrumInstDecay(n) => inst(n).map_or(1.0, |i| i.decay),
        Control::DrumSelLevel => d.inst[sel].level,
        Control::DrumSelTune => d.inst[sel].tune,
        Control::DrumSelDecay => d.inst[sel].decay,
        // The current pattern lit, the one waiting to start blinking.
        Control::DrumPattern(n) => idx(n, PATTERNS).map_or(0.0, |p| {
            if d.queued == Some(p as u8) { b(beat_phase < 0.5) } else { b(usize::from(d.current) == p) }
        }),
        // Numbers for two-digit displays.
        Control::DrumPatternSelect => f32::from(d.queued.unwrap_or(d.current)) + 1.0,
        Control::DrumLength => f32::from(pat.length),
        Control::DrumSwing => pat.swing,
        Control::DrumLevel => d.channel.volume,
        Control::DrumFilter => d.channel.filter,
        Control::DrumFxAssign(n) => idx(n, 2).map_or(0.0, |i| b(d.channel.fx_assign[i])),
        Control::DrumPfl => b(d.channel.pfl),
        Control::DrumMeter => d.channel.meter[0].max(d.channel.meter[1]),
        other => other.default_value(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rille_core::drums::Pattern;
    use rille_engine::LOOP_SIZES;
    use rille_midi::hid::{DISPLAY_LOOP_SIZE, display_text};

    #[test]
    fn display_codes_cover_the_loop_sizes() {
        for (i, size) in LOOP_SIZES.iter().enumerate() {
            let text = display_text(DISPLAY_LOOP_SIZE + i as u8);
            assert_eq!(text.parse::<f64>().ok(), Some(*size), "{text}");
        }
    }

    #[test]
    fn drum_leds() {
        let mut d = DrumState::default();
        let mut p = Pattern::default();
        p.set(1, 0, true);
        p.set_accent(1, 4, true);
        d.patterns[2] = p;
        d.current = 2;
        d.selected = 1;
        d.playing = true;
        d.step = Some(4);
        d.queued = Some(5);
        let v = |c| drum_value(c, &d, 0.25);
        assert_eq!(v(Control::DrumStep(1)), 1.0);
        assert_eq!(v(Control::DrumStep(5)), 0.0, "the playhead inverts a step that is on");
        assert_eq!(v(Control::DrumStep(6)), 0.0);
        assert_eq!(v(Control::DrumStepLed(5)), f32::from(led_code(0, true)), "white playhead");
        assert_eq!(v(Control::DrumStepLed(1)), f32::from(led_code(INST_COLORS[1], false)));
        assert_eq!(v(Control::DrumCell(16 + 5)), 1.0);
        assert_eq!(v(Control::DrumInst(2)), 1.0);
        assert_eq!(v(Control::DrumInstLed(2)), f32::from(led_code(INST_COLORS[1], true)));
        assert_eq!(v(Control::DrumInstLed(1)), 0.0, "no steps, not selected");
        assert_eq!(v(Control::DrumPattern(3)), 1.0);
        assert_eq!(v(Control::DrumPattern(6)), 1.0, "queued blinks: on in the first half of the beat");
        assert_eq!(drum_value(Control::DrumPattern(6), &d, 0.75), 0.0);
        assert_eq!(v(Control::DrumPatternSelect), 6.0);
        assert_eq!(v(Control::DrumStep(17)), 0.0);
    }
}
