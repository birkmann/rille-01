//! Control identifiers shared by the UI, keyboard shortcuts and MIDI mappings.
//!
//! A [`ControlTarget`] names one control on one unit (a deck, an FX unit, the
//! drum machine or the global section). Its string form is used in mapping
//! files: `deck.A.play`, `deck.B.hotcue.3`, `fx.1.knob.2`, `drum.step.5`,
//! `global.crossfader`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Which kind of unit a control belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Deck transport and the deck's mixer channel.
    Deck,
    Fx,
    /// The drum machine (one unit).
    Drum,
    Global,
}

/// How a control is driven.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControlKind {
    /// Press and release events; the engine decides toggle vs hold semantics.
    Button,
    /// Absolute value in `0..=1`.
    Continuous,
    /// Signed increments (encoders, jog wheels, browser scrolling).
    Relative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    // Deck transport
    Play,
    Cue,
    Cup,
    Sync,
    Master,
    Keylock,
    Flux,
    Reverse,
    /// Metronome click on every grid beat (accent on bar 1), to check the grid by ear.
    Tick,
    /// Hotcue slot 1..=8: press jumps (or sets an empty slot).
    Hotcue(u8),
    HotcueDelete(u8),
    LoopIn,
    LoopOut,
    /// Auto loop with the selected size, or exit the active loop.
    LoopToggle,
    LoopSizeDown,
    LoopSizeUp,
    LoopHalve,
    LoopDouble,
    BeatjumpBack,
    BeatjumpForward,
    /// Back to the start of the track; a stopped deck stays stopped.
    JumpStart,
    TempoBendDown,
    TempoBendUp,
    TempoReset,
    /// Tempo fader, 0.5 = no change.
    Tempo,
    /// Position in the track, 0..=1.
    Seek,
    Jog,
    JogTouch,
    LoadSelected,
    Eject,
    // Deck's mixer channel
    Gain,
    EqHi,
    EqMid,
    EqLo,
    /// Silences the band while held (touch-sensitive EQ knobs).
    EqHiKill,
    EqMidKill,
    EqLoKill,
    /// Filter knob, 0.5 = off, below = low-pass, above = high-pass.
    Filter,
    /// Loop roll while held, shorter the further the filter knob is from
    /// the centre (none near it); playback returns to where it would be.
    FilterRoll,
    Volume,
    /// Key shift knob, 0.5 = original key.
    KeyShift,
    Pfl,
    /// Channel level before the fader (after gain, EQ and filter; peak,
    /// 1.0 = 0 dBFS). Read-only: for controller meters, never an input.
    Meter,
    /// Assign the channel to FX unit 1..=4.
    FxAssign(u8),
    // FX unit
    FxOn,
    FxDryWet,
    /// Amount of the effect in slot 1..=3 (group mode).
    FxKnob(u8),
    /// Switches the effect in slot 1..=3 on or off.
    FxButton(u8),
    /// First parameter of the effect in slot 1..=3 (delay time, filter
    /// cutoff, …).
    FxParam(u8),
    /// Step the effect in slot 1..=3.
    FxSelect(u8),
    // Remix deck (a deck switched to remix mode; track decks ignore these).
    // Read back for LEDs: cells and pads give a `rille_core::remix::led_code`,
    // `remix_page` the page number (1..=4), stop buttons whether the slot plays.
    /// Trigger cell 1..=64 (`slot * 16 + row + 1`).
    RemixCell(u8),
    /// Trigger pad 1..=16 of the visible page (4×4, row by row from the top left).
    RemixPad(u8),
    /// Empty the pad's cell.
    RemixPadDelete(u8),
    /// Capture a loop from the capture source deck into the pad's cell.
    RemixPadCapture(u8),
    /// Switch the pad's cell between loop and one-shot.
    RemixPadType(u8),
    /// Load the track selected in the browser into the pad's cell.
    RemixPadLoad(u8),
    /// Stop slot 1..=4.
    RemixStop(u8),
    /// Mute slot 1..=4 (toggle); it keeps playing silently.
    RemixMute(u8),
    /// Volume of slot 1..=4.
    RemixVolume(u8),
    /// Filter of slot 1..=4, 0.5 = off, like [`Control::Filter`].
    RemixFilter(u8),
    /// Page of the pad grid (relative).
    RemixPage,
    /// Start triggered samples on the next quantize boundary.
    RemixQuantize,
    /// Quantize size (relative), 1/4 beat … 8 beats.
    RemixQuantizeSize,
    /// Deck that CAPTURE takes loops from (relative).
    RemixCaptureSource,
    // Global
    Crossfader,
    /// 0 = slow fade (both sides dip in the middle), 0.5 = both at full level
    /// in the middle, 1 = sharp cut at the ends (scratching).
    CrossfaderCurve,
    /// Swaps the crossfader's sides while held (a hamster switch).
    CrossfaderReverse,
    MainLevel,
    /// Main output level of side 1 (left) or 2 (right), like [`Control::Meter`].
    /// Read-only.
    MainMeter(u8),
    CueMix,
    CueVolume,
    Quantize,
    Snap,
    Limiter,
    ClockTempo,
    BrowserScroll,
    BrowserTreeScroll,
    BrowserToggleNode,
    /// Start or stop recording the main mix.
    Record,
    /// Level of stem 1..=4 (drums, bass, other, vocals) of a deck's track.
    StemVolume(u8),
    /// Mutes or unmutes stem 1..=4 (toggles on press).
    StemMute(u8),
    // Drum machine (see `rille_core::drums`). Steps, accents and the
    // instrument's knobs act on the selected instrument; cells address any
    // instrument's step directly. Step and cell controls toggle on press and
    // set on an absolute value (on from 0.5).
    /// Start or stop the sequencer (it starts in phase with the master clock).
    DrumPlay,
    /// Live recording: triggers while playing write the nearest step.
    DrumRecord,
    /// Step 1..=16 of the selected instrument. Read back: lit when on,
    /// inverted under the playhead (a running light).
    DrumStep(u8),
    /// Accent on step 1..=16 of the selected instrument.
    DrumAccent(u8),
    /// Read-only pad colour of step 1..=16 (`rille_core::remix::led_code`):
    /// the instrument's colour when on, white under the playhead.
    DrumStepLed(u8),
    /// Step of any instrument, 1..=128 (`instrument * 16 + step + 1`).
    DrumCell(u8),
    DrumCellAccent(u8),
    /// Select instrument 1..=8.
    DrumInst(u8),
    /// Read-only pad colour of instrument 1..=8: bright when selected, dim
    /// when it has steps.
    DrumInstLed(u8),
    /// Play instrument 1..=8 now (and record it while REC is on).
    DrumTrigger(u8),
    /// Select the next or previous instrument (relative).
    DrumInstSelect,
    DrumInstMute(u8),
    DrumInstLevel(u8),
    /// Pitch of instrument 1..=8, 0.5 = original, ±12 semitones.
    DrumInstTune(u8),
    /// Length of instrument 1..=8's sound, 1.0 = the whole sample.
    DrumInstDecay(u8),
    /// Level, tune and decay of the selected instrument.
    DrumSelLevel,
    DrumSelTune,
    DrumSelDecay,
    /// Select pattern 1..=16; while playing it starts when the current one
    /// comes round.
    DrumPattern(u8),
    /// Next or previous pattern (relative).
    DrumPatternSelect,
    /// Pattern length in steps (relative).
    DrumLength,
    DrumSwing,
    /// Clear the selected instrument's steps.
    DrumClear,
    DrumClearPattern,
    /// Next or previous kit (relative).
    DrumKitSelect,
    /// Drum channel fader.
    DrumLevel,
    /// Drum channel filter, 0.5 = off.
    DrumFilter,
    /// Send the drums through FX unit 1..=2.
    DrumFxAssign(u8),
    /// Drums in the headphones.
    DrumPfl,
    /// Drum channel level, like [`Control::Meter`]. Read-only.
    DrumMeter,
    /// Show or hide the drum machine panel.
    DrumShow,
    /// Solo instrument 1..=8 (toggles); while any is soloed, only soloed
    /// instruments sound.
    DrumInstSolo(u8),
    /// Clear instrument 1..=8's steps in the current pattern.
    DrumInstClear(u8),
    /// Held: rolls instrument 1..=8 at the repeat rate, in time with the
    /// clock (and records it while REC is on).
    DrumRepeat(u8),
    /// Note repeat rate (relative), see `rille_core::drums::REPEAT_RATES`.
    DrumRepeatRate,
    /// Pattern length 1..=16 steps.
    DrumLengthSet(u8),
    /// Copy the current pattern to pattern 1..=16 and switch to it.
    DrumPatternCopy(u8),
    /// Move the selected instrument's steps by whole steps (relative, +1 =
    /// one step later), wrapping round the pattern length.
    DrumNudge,
    /// Undo or redo the last pattern edit.
    DrumUndo,
    DrumRedo,
    /// Copy the current pattern; paste it over the current pattern.
    DrumCopy,
    DrumPaste,
    /// Closed hi-hat cuts the open one (toggles).
    DrumChoke,
    /// Stopped: start on the next downbeat. Playing: stop at the end of the
    /// bar.
    DrumPlayBar,
    /// Load kit 1..=16 of the kit list (factory kits first).
    DrumKit(u8),
    /// Read-only pad colours (`rille_core::remix::led_code`): pattern 1..=16
    /// (bright = current, blinking = queued, dim = has steps), instrument
    /// 1..=8 as a trigger pad (flashes on a hit, off when it cannot be
    /// heard), as a mute pad (dim when it sounds, bright when soloed, off
    /// when muted), pattern length 1..=16 (lit up to the length), kit
    /// 1..=16 (bright = loaded, dim = available).
    DrumPatternLed(u8),
    DrumTriggerLed(u8),
    DrumMuteLed(u8),
    DrumLengthLed(u8),
    DrumKitLed(u8),
    /// Read-only colour of the selected instrument (bright).
    DrumSelLed,
    /// Count-in: recording starts on the next downbeat (the drums start
    /// there too if they were stopped). Again: cancel.
    DrumCountIn,
    /// Replace recording (toggles): while recording, the first hit of an
    /// instrument clears its old steps.
    DrumReplace,
    /// Load the track selected in the library as instrument 1..=8's sound.
    DrumLoadSelected(u8),
}

impl Control {
    pub fn scope(self) -> Scope {
        use Control::*;
        match self {
            FxOn | FxDryWet | FxKnob(_) | FxButton(_) | FxParam(_) | FxSelect(_) => Scope::Fx,
            Crossfader | CrossfaderCurve | CrossfaderReverse | MainLevel | MainMeter(_) | CueMix | CueVolume
            | Quantize | Snap | Limiter | ClockTempo | BrowserScroll | BrowserTreeScroll | BrowserToggleNode
            | Record => Scope::Global,
            c if c.is_drum() => Scope::Drum,
            _ => Scope::Deck,
        }
    }

    pub fn kind(self) -> ControlKind {
        use Control::*;
        match self {
            Tempo | Seek | Gain | EqHi | EqMid | EqLo | Filter | Volume | KeyShift | Meter | FxDryWet | FxKnob(_)
            | FxParam(_) | Crossfader | CrossfaderCurve | MainLevel | MainMeter(_) | CueMix | CueVolume
            | ClockTempo | RemixVolume(_) | RemixFilter(_) | StemVolume(_) | DrumInstLevel(_) | DrumInstTune(_)
            | DrumInstDecay(_) | DrumSelLevel | DrumSelTune | DrumSelDecay | DrumSwing | DrumLevel | DrumFilter
            | DrumMeter => ControlKind::Continuous,
            Jog | FxSelect(_) | BrowserScroll | BrowserTreeScroll | RemixPage | RemixQuantizeSize
            | RemixCaptureSource | DrumInstSelect | DrumPatternSelect | DrumLength | DrumKitSelect | DrumRepeatRate
            | DrumNudge => ControlKind::Relative,
            _ => ControlKind::Button,
        }
    }

    /// A remix deck control (cells, pads, slots, page…).
    pub fn is_remix(self) -> bool {
        use Control::*;
        matches!(
            self,
            RemixCell(_)
                | RemixPad(_)
                | RemixPadDelete(_)
                | RemixPadCapture(_)
                | RemixPadType(_)
                | RemixPadLoad(_)
                | RemixStop(_)
                | RemixMute(_)
                | RemixVolume(_)
                | RemixFilter(_)
                | RemixPage
                | RemixQuantize
                | RemixQuantizeSize
                | RemixCaptureSource
        )
    }

    /// A drum machine control.
    pub fn is_drum(self) -> bool {
        use Control::*;
        matches!(
            self,
            DrumPlay
                | DrumRecord
                | DrumStep(_)
                | DrumAccent(_)
                | DrumStepLed(_)
                | DrumCell(_)
                | DrumCellAccent(_)
                | DrumInst(_)
                | DrumInstLed(_)
                | DrumTrigger(_)
                | DrumInstSelect
                | DrumInstMute(_)
                | DrumInstLevel(_)
                | DrumInstTune(_)
                | DrumInstDecay(_)
                | DrumSelLevel
                | DrumSelTune
                | DrumSelDecay
                | DrumPattern(_)
                | DrumPatternSelect
                | DrumLength
                | DrumSwing
                | DrumClear
                | DrumClearPattern
                | DrumKitSelect
                | DrumLevel
                | DrumFilter
                | DrumFxAssign(_)
                | DrumPfl
                | DrumMeter
                | DrumShow
                | DrumInstSolo(_)
                | DrumInstClear(_)
                | DrumRepeat(_)
                | DrumRepeatRate
                | DrumLengthSet(_)
                | DrumPatternCopy(_)
                | DrumNudge
                | DrumUndo
                | DrumRedo
                | DrumCopy
                | DrumPaste
                | DrumChoke
                | DrumPlayBar
                | DrumKit(_)
                | DrumPatternLed(_)
                | DrumTriggerLed(_)
                | DrumMuteLed(_)
                | DrumLengthLed(_)
                | DrumKitLed(_)
                | DrumSelLed
                | DrumCountIn
                | DrumReplace
                | DrumLoadSelected(_)
        )
    }

    /// Whether hardware or the UI can drive it; false for read-only state
    /// such as [`Control::Meter`].
    pub fn is_input(self) -> bool {
        use Control::*;
        !matches!(
            self,
            Meter
                | MainMeter(_)
                | DrumMeter
                | DrumStepLed(_)
                | DrumInstLed(_)
                | DrumPatternLed(_)
                | DrumTriggerLed(_)
                | DrumMuteLed(_)
                | DrumLengthLed(_)
                | DrumKitLed(_)
                | DrumSelLed
        )
    }

    /// Resting value of continuous controls (used for reset and soft-takeover).
    pub fn default_value(self) -> f32 {
        use Control::*;
        match self {
            Tempo | Gain | EqHi | EqMid | EqLo | Filter | KeyShift | Crossfader | CrossfaderCurve | CueMix
            | RemixFilter(_) | DrumInstTune(_) | DrumSelTune | DrumFilter => 0.5,
            Volume | RemixVolume(_) | StemVolume(_) | DrumInstLevel(_) | DrumSelLevel | DrumInstDecay(_)
            | DrumSelDecay => 1.0,
            MainLevel | CueVolume | DrumLevel => 0.8,
            _ => 0.0,
        }
    }

    /// Every control, for UIs listing mappable targets.
    pub fn all() -> Vec<Control> {
        use Control::*;
        let mut v = vec![
            Play,
            Cue,
            Cup,
            Sync,
            Master,
            Keylock,
            Flux,
            Reverse,
            Tick,
            LoopIn,
            LoopOut,
            LoopToggle,
            LoopSizeDown,
            LoopSizeUp,
            LoopHalve,
            LoopDouble,
            BeatjumpBack,
            BeatjumpForward,
            JumpStart,
            TempoBendDown,
            TempoBendUp,
            TempoReset,
            Tempo,
            Seek,
            Jog,
            JogTouch,
            LoadSelected,
            Eject,
            Gain,
            EqHi,
            EqMid,
            EqLo,
            EqHiKill,
            EqMidKill,
            EqLoKill,
            Filter,
            FilterRoll,
            Volume,
            KeyShift,
            Pfl,
            Meter,
            FxOn,
            FxDryWet,
            Crossfader,
            CrossfaderCurve,
            CrossfaderReverse,
            MainLevel,
            CueMix,
            CueVolume,
            Quantize,
            Snap,
            Limiter,
            ClockTempo,
            BrowserScroll,
            BrowserTreeScroll,
            BrowserToggleNode,
            Record,
            RemixPage,
            RemixQuantize,
            RemixQuantizeSize,
            RemixCaptureSource,
        ];
        v.extend((1..=4).map(StemVolume));
        v.extend((1..=4).map(StemMute));
        v.extend((1..=8).map(Hotcue));
        v.extend((1..=8).map(HotcueDelete));
        v.extend((1..=2).map(FxAssign));
        v.extend((1..=3).map(FxKnob));
        v.extend((1..=3).map(FxButton));
        v.extend((1..=3).map(FxParam));
        v.extend((1..=3).map(FxSelect));
        v.extend((1..=2).map(MainMeter));
        v.extend((1..=crate::remix::CELLS as u8).map(RemixCell));
        for f in [RemixPad, RemixPadDelete, RemixPadCapture, RemixPadType, RemixPadLoad] {
            v.extend((1..=(crate::remix::SLOTS * crate::remix::PAGE_ROWS) as u8).map(f));
        }
        for f in [RemixStop, RemixMute, RemixVolume, RemixFilter] {
            v.extend((1..=crate::remix::SLOTS as u8).map(f));
        }
        use crate::drums::{CELLS, INSTRUMENTS, PATTERNS, STEPS};
        v.extend([
            DrumPlay,
            DrumRecord,
            DrumInstSelect,
            DrumSelLevel,
            DrumSelTune,
            DrumSelDecay,
            DrumPatternSelect,
            DrumLength,
            DrumSwing,
            DrumClear,
            DrumClearPattern,
            DrumKitSelect,
            DrumLevel,
            DrumFilter,
            DrumPfl,
            DrumMeter,
            DrumShow,
            DrumRepeatRate,
            DrumNudge,
            DrumUndo,
            DrumRedo,
            DrumCopy,
            DrumPaste,
            DrumChoke,
            DrumPlayBar,
            DrumSelLed,
            DrumCountIn,
            DrumReplace,
        ]);
        for f in [DrumStep, DrumAccent, DrumStepLed] {
            v.extend((1..=STEPS as u8).map(f));
        }
        for f in [DrumCell, DrumCellAccent] {
            v.extend((1..=CELLS as u8).map(f));
        }
        for f in [
            DrumInst,
            DrumInstLed,
            DrumTrigger,
            DrumInstMute,
            DrumInstLevel,
            DrumInstTune,
            DrumInstDecay,
            DrumInstSolo,
            DrumInstClear,
            DrumRepeat,
            DrumTriggerLed,
            DrumMuteLed,
            DrumLoadSelected,
        ] {
            v.extend((1..=INSTRUMENTS as u8).map(f));
        }
        for f in [DrumPattern, DrumPatternCopy, DrumPatternLed] {
            v.extend((1..=PATTERNS as u8).map(f));
        }
        for f in [DrumLengthSet, DrumLengthLed] {
            v.extend((1..=STEPS as u8).map(f));
        }
        for f in [DrumKit, DrumKitLed] {
            v.extend((1..=crate::drums::KIT_PADS as u8).map(f));
        }
        v.extend((1..=2).map(DrumFxAssign));
        v
    }

    fn name(self) -> (&'static str, Option<u8>) {
        use Control::*;
        match self {
            Play => ("play", None),
            Cue => ("cue", None),
            Cup => ("cup", None),
            Sync => ("sync", None),
            Master => ("master", None),
            Keylock => ("keylock", None),
            Flux => ("flux", None),
            Reverse => ("reverse", None),
            Tick => ("tick", None),
            Hotcue(n) => ("hotcue", Some(n)),
            HotcueDelete(n) => ("hotcue_delete", Some(n)),
            LoopIn => ("loop_in", None),
            LoopOut => ("loop_out", None),
            LoopToggle => ("loop_toggle", None),
            LoopSizeDown => ("loop_size_down", None),
            LoopSizeUp => ("loop_size_up", None),
            LoopHalve => ("loop_halve", None),
            LoopDouble => ("loop_double", None),
            BeatjumpBack => ("beatjump_back", None),
            BeatjumpForward => ("beatjump_forward", None),
            JumpStart => ("jump_start", None),
            TempoBendDown => ("tempo_bend_down", None),
            TempoBendUp => ("tempo_bend_up", None),
            TempoReset => ("tempo_reset", None),
            Tempo => ("tempo", None),
            Seek => ("seek", None),
            Jog => ("jog", None),
            JogTouch => ("jog_touch", None),
            LoadSelected => ("load_selected", None),
            Eject => ("eject", None),
            Gain => ("gain", None),
            EqHi => ("eq_hi", None),
            EqMid => ("eq_mid", None),
            EqLo => ("eq_lo", None),
            EqHiKill => ("eq_hi_kill", None),
            EqMidKill => ("eq_mid_kill", None),
            EqLoKill => ("eq_lo_kill", None),
            Filter => ("filter", None),
            FilterRoll => ("filter_roll", None),
            Volume => ("volume", None),
            KeyShift => ("key_shift", None),
            Pfl => ("pfl", None),
            Meter => ("meter", None),
            FxAssign(n) => ("fx_assign", Some(n)),
            FxOn => ("on", None),
            FxDryWet => ("dry_wet", None),
            FxKnob(n) => ("knob", Some(n)),
            FxButton(n) => ("button", Some(n)),
            FxParam(n) => ("param", Some(n)),
            FxSelect(n) => ("select", Some(n)),
            Crossfader => ("crossfader", None),
            CrossfaderCurve => ("crossfader_curve", None),
            CrossfaderReverse => ("crossfader_reverse", None),
            MainLevel => ("main_level", None),
            MainMeter(n) => ("main_meter", Some(n)),
            CueMix => ("cue_mix", None),
            CueVolume => ("cue_volume", None),
            Quantize => ("quantize", None),
            Snap => ("snap", None),
            Limiter => ("limiter", None),
            ClockTempo => ("clock_tempo", None),
            BrowserScroll => ("scroll", None),
            BrowserTreeScroll => ("tree_scroll", None),
            BrowserToggleNode => ("toggle_node", None),
            Record => ("record", None),
            StemVolume(n) => ("stem_volume", Some(n)),
            StemMute(n) => ("stem_mute", Some(n)),
            RemixCell(n) => ("remix_cell", Some(n)),
            RemixPad(n) => ("remix_pad", Some(n)),
            RemixPadDelete(n) => ("remix_pad_delete", Some(n)),
            RemixPadCapture(n) => ("remix_pad_capture", Some(n)),
            RemixPadType(n) => ("remix_pad_type", Some(n)),
            RemixPadLoad(n) => ("remix_pad_load", Some(n)),
            RemixStop(n) => ("remix_stop", Some(n)),
            RemixMute(n) => ("remix_mute", Some(n)),
            RemixVolume(n) => ("remix_volume", Some(n)),
            RemixFilter(n) => ("remix_filter", Some(n)),
            RemixPage => ("remix_page", None),
            RemixQuantize => ("remix_quantize", None),
            RemixQuantizeSize => ("remix_quantize_size", None),
            RemixCaptureSource => ("remix_capture_source", None),
            DrumPlay => ("play", None),
            DrumRecord => ("record", None),
            DrumStep(n) => ("step", Some(n)),
            DrumAccent(n) => ("accent", Some(n)),
            DrumStepLed(n) => ("step_led", Some(n)),
            DrumCell(n) => ("cell", Some(n)),
            DrumCellAccent(n) => ("cell_accent", Some(n)),
            DrumInst(n) => ("inst", Some(n)),
            DrumInstLed(n) => ("inst_led", Some(n)),
            DrumTrigger(n) => ("trigger", Some(n)),
            DrumInstSelect => ("inst_select", None),
            DrumInstMute(n) => ("inst_mute", Some(n)),
            DrumInstLevel(n) => ("inst_level", Some(n)),
            DrumInstTune(n) => ("inst_tune", Some(n)),
            DrumInstDecay(n) => ("inst_decay", Some(n)),
            DrumSelLevel => ("sel_level", None),
            DrumSelTune => ("sel_tune", None),
            DrumSelDecay => ("sel_decay", None),
            DrumPattern(n) => ("pattern", Some(n)),
            DrumPatternSelect => ("pattern_select", None),
            DrumLength => ("length", None),
            DrumSwing => ("swing", None),
            DrumClear => ("clear", None),
            DrumClearPattern => ("clear_pattern", None),
            DrumKitSelect => ("kit_select", None),
            DrumLevel => ("level", None),
            DrumFilter => ("filter", None),
            DrumFxAssign(n) => ("fx_assign", Some(n)),
            DrumPfl => ("pfl", None),
            DrumMeter => ("meter", None),
            DrumShow => ("show", None),
            DrumInstSolo(n) => ("inst_solo", Some(n)),
            DrumInstClear(n) => ("inst_clear", Some(n)),
            DrumRepeat(n) => ("repeat", Some(n)),
            DrumRepeatRate => ("repeat_rate", None),
            DrumLengthSet(n) => ("length_set", Some(n)),
            DrumPatternCopy(n) => ("pattern_copy", Some(n)),
            DrumNudge => ("nudge", None),
            DrumUndo => ("undo", None),
            DrumRedo => ("redo", None),
            DrumCopy => ("copy", None),
            DrumPaste => ("paste", None),
            DrumChoke => ("choke", None),
            DrumPlayBar => ("play_bar", None),
            DrumKit(n) => ("kit", Some(n)),
            DrumPatternLed(n) => ("pattern_led", Some(n)),
            DrumTriggerLed(n) => ("trigger_led", Some(n)),
            DrumMuteLed(n) => ("mute_led", Some(n)),
            DrumLengthLed(n) => ("length_led", Some(n)),
            DrumKitLed(n) => ("kit_led", Some(n)),
            DrumSelLed => ("sel_led", None),
            DrumCountIn => ("count_in", None),
            DrumReplace => ("replace", None),
            DrumLoadSelected(n) => ("load_selected", Some(n)),
        }
    }

    /// Names repeat across scopes (`deck.A.play`, `drum.play`).
    fn from_name(scope: Scope, name: &str, n: Option<u8>) -> Option<Control> {
        Control::all().into_iter().find(|c| c.scope() == scope && c.name() == (name, n))
    }
}

/// A control on a specific unit. `unit` is the deck index (0 = A) for deck
/// controls, the FX unit index (0 = unit 1) for FX controls, and 0 for the
/// drum machine and global.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ControlTarget {
    pub control: Control,
    pub unit: u8,
}

impl ControlTarget {
    pub fn deck(deck: u8, control: Control) -> Self {
        Self { control, unit: deck }
    }

    pub fn fx(unit: u8, control: Control) -> Self {
        Self { control, unit }
    }

    pub fn global(control: Control) -> Self {
        Self { control, unit: 0 }
    }

    pub fn drum(control: Control) -> Self {
        Self { control, unit: 0 }
    }
}

impl fmt::Display for ControlTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, n) = self.control.name();
        match self.control.scope() {
            Scope::Deck => write!(f, "deck.{}.{name}", char::from(b'A' + self.unit))?,
            Scope::Fx => write!(f, "fx.{}.{name}", self.unit + 1)?,
            Scope::Drum => write!(f, "drum.{name}")?,
            Scope::Global => write!(f, "global.{name}")?,
        }
        if let Some(n) = n {
            write!(f, ".{n}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseControlError(pub String);

impl fmt::Display for ParseControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown control target '{}'", self.0)
    }
}

impl std::error::Error for ParseControlError {}

impl FromStr for ControlTarget {
    type Err = ParseControlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseControlError(s.to_owned());
        let parts: Vec<&str> = s.split('.').collect();
        let (scope, unit, rest) = match parts.as_slice() {
            ["deck", d, rest @ ..] => {
                let c = d.chars().next().filter(|c| d.len() == 1 && ('A'..='D').contains(c)).ok_or_else(err)?;
                (Scope::Deck, c as u8 - b'A', rest)
            }
            ["fx", u, rest @ ..] => {
                let u: u8 = u.parse().ok().filter(|u| (1..=4).contains(u)).ok_or_else(err)?;
                (Scope::Fx, u - 1, rest)
            }
            ["drum", rest @ ..] => (Scope::Drum, 0, rest),
            ["global", rest @ ..] => (Scope::Global, 0, rest),
            _ => return Err(err()),
        };
        let (name, n) = match rest {
            [name] => (*name, None),
            [name, n] => (*name, Some(n.parse::<u8>().map_err(|_| err())?)),
            _ => return Err(err()),
        };
        let control = Control::from_name(scope, name, n).ok_or_else(err)?;
        Ok(Self { control, unit })
    }
}

impl Serialize for ControlTarget {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ControlTarget {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A value sent to a control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlValue {
    /// Button pressed (`true`) or released (`false`).
    Press(bool),
    /// Absolute position `0..=1`.
    Absolute(f32),
    /// Relative change; for jog wheels in revolutions, for encoders in steps.
    Delta(f32),
    /// A press with a velocity `0..=1` (a pressure-sensitive pad); its
    /// release is `Press(false)`. Controls that ignore velocity treat it as
    /// `Press(true)`.
    Hit(f32),
}

impl ControlValue {
    /// A button going down, with or without velocity.
    pub fn is_press(self) -> bool {
        matches!(self, ControlValue::Press(true) | ControlValue::Hit(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlEvent {
    pub target: ControlTarget,
    pub value: ControlValue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_roundtrip_for_all_controls() {
        for c in Control::all() {
            let units: &[u8] = match c.scope() {
                Scope::Deck => &[0, 3],
                Scope::Fx => &[0, 1],
                Scope::Drum | Scope::Global => &[0],
            };
            for &unit in units {
                let t = ControlTarget { control: c, unit };
                let s = t.to_string();
                assert_eq!(s.parse::<ControlTarget>(), Ok(t), "{s}");
            }
        }
    }

    #[test]
    fn examples() {
        assert_eq!(ControlTarget::deck(1, Control::Hotcue(3)).to_string(), "deck.B.hotcue.3");
        assert_eq!(ControlTarget::fx(0, Control::FxKnob(2)).to_string(), "fx.1.knob.2");
        assert_eq!(ControlTarget::global(Control::Crossfader).to_string(), "global.crossfader");
        assert!("deck.E.play".parse::<ControlTarget>().is_err());
        assert!("global.play".parse::<ControlTarget>().is_err());
        assert!("deck.A.hotcue".parse::<ControlTarget>().is_err());
        assert_eq!(ControlTarget::deck(2, Control::RemixCell(64)).to_string(), "deck.C.remix_cell.64");
        assert_eq!("deck.D.remix_volume.4".parse(), Ok(ControlTarget::deck(3, Control::RemixVolume(4))));
        assert!("deck.C.remix_pad.17".parse::<ControlTarget>().is_err());
    }

    #[test]
    fn drum_targets() {
        assert_eq!(ControlTarget::drum(Control::DrumStep(5)).to_string(), "drum.step.5");
        // Names shared with deck and global controls resolve by scope.
        assert_eq!("drum.play".parse(), Ok(ControlTarget::drum(Control::DrumPlay)));
        assert_eq!("deck.A.play".parse(), Ok(ControlTarget::deck(0, Control::Play)));
        assert_eq!("drum.filter".parse(), Ok(ControlTarget::drum(Control::DrumFilter)));
        assert_eq!("deck.B.filter".parse(), Ok(ControlTarget::deck(1, Control::Filter)));
        assert_eq!("drum.record".parse(), Ok(ControlTarget::drum(Control::DrumRecord)));
        assert_eq!("global.record".parse(), Ok(ControlTarget::global(Control::Record)));
        assert_eq!("drum.cell.128".parse(), Ok(ControlTarget::drum(Control::DrumCell(128))));
        assert!("drum.cell.129".parse::<ControlTarget>().is_err());
        assert!("drum.step.17".parse::<ControlTarget>().is_err());
        assert!("drum.A.play".parse::<ControlTarget>().is_err());
        assert!(!Control::DrumStepLed(1).is_input() && Control::DrumStep(1).is_input());
        assert_eq!(Control::DrumPatternSelect.kind(), ControlKind::Relative);
        assert_eq!(Control::DrumSelTune.kind(), ControlKind::Continuous);
        assert!(Control::all().iter().all(|c| c.is_drum() == (c.scope() == Scope::Drum)));
    }
}
