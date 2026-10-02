//! Current control values from the engine snapshot, for MIDI soft-takeover
//! and controller LEDs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rille_core::remix::{CELLS, SLOTS, led_code, pad_cell};
use rille_core::{Control, ControlTarget};
use rille_engine::{EngineHandle, MAX_DECKS, REMIX_QUANT_SIZES, RemixState};
use rille_midi::ValueSource;
use rille_midi::hid::DISPLAY_LOOP_SIZE;

use crate::engine_slot::EngineSlot;

/// The engine's state, plus whether the main mix is being recorded.
pub struct SnapshotValues(pub Arc<EngineSlot>, pub Arc<AtomicBool>);

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
            return b(self.1.load(Ordering::Relaxed));
        }
        let Some(engine): Option<Arc<EngineHandle>> = self.0.get() else { return t.control.default_value() };
        let s = engine.snapshot();
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
        self.0.get().map_or(0.0, |e| e.snapshot().clock_beat.rem_euclid(1.0))
    }
}

#[cfg(test)]
mod tests {
    use rille_engine::LOOP_SIZES;
    use rille_midi::hid::{DISPLAY_LOOP_SIZE, display_text};

    #[test]
    fn display_codes_cover_the_loop_sizes() {
        for (i, size) in LOOP_SIZES.iter().enumerate() {
            let text = display_text(DISPLAY_LOOP_SIZE + i as u8);
            assert_eq!(text.parse::<f64>().ok(), Some(*size), "{text}");
        }
    }
}
