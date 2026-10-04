//! The audio-thread side: renders decks, keeps them in sync, mixes.
//!
//! Sync: a master clock counts beats. While a deck leads, the clock sits on
//! the beat you hear from that deck (re-anchored every block, so seeks,
//! jumps and nudges of the master carry over); it only free-runs at the
//! master's tempo while the master scratches, reverses or loops less than a
//! beat. Every synced deck computes, each block, where its grid must be at
//! the end of the block: its current beat plus the clock's travel, plus a
//! fraction of the remaining phase error. The speed follows from its grid,
//! so constant, piecewise and live grids all lock the same way and the error
//! cannot accumulate. A larger error (after a jump of either deck) is fixed
//! at once by a crossfaded jump into phase, never by seconds of tempo pull.

use std::time::Instant;

use rille_core::quantize::wrap_phase_error;
use rille_core::{BeatClock, Control, ControlEvent, ControlValue, Scope};
use rille_dsp::{FxCtx, FxUnit, PeakLimiter, PeakMeter, SincTable, SoftClip, flush_denormals};

use crate::deck::{Deck, Modes, RenderCtx, filter_roll_beats};
use crate::drums::DrumMachine;
use crate::mixer::{Strip, crossfader_gain};
use crate::snapshot::{ChannelState, DeckState, FxState, Snapshot};
use crate::types::{Command, Event, FX_UNITS, Garbage, LOOP_SIZES, MAX_DECKS, Recorder, Settings};

/// Phase errors are corrected over roughly this time.
const PHASE_CORRECTION_SECS: f64 = 0.08;
/// Largest speed change sync may apply to fix phase (relative).
const MAX_PHASE_PULL: f64 = 0.04;
/// Largest speed change while following a nudge of a synced deck.
const NUDGE_PULL: f64 = 0.3;
/// Phase errors of at least this many beats are fixed by a crossfaded jump.
const REALIGN_BEATS: f64 = 0.004;
/// A deck jumps into phase at most this often (seconds).
const REALIGN_INTERVAL_SECS: f64 = 0.05;

struct Clock {
    /// Deck leading the tempo; `None` = internal clock.
    master: Option<u8>,
    /// Pick a master automatically when none leads.
    auto: bool,
    /// The user chose the master deck (MASTER button); keep it.
    explicit: bool,
    bpm: f64,
    beat: f64,
    /// Clock beat of a downbeat of the leading track: bars (and drum
    /// patterns) count from here.
    bar_origin: f64,
    /// Leader the origin was taken from.
    origin_leader: Option<u8>,
    /// A different origin the leader suggests, and the clock beat since
    /// when (it must hold a beat before it is taken: loops wrap, rolls end).
    origin_shift: Option<(f64, f64)>,
}

pub struct Engine {
    sr: f64,
    max_block: usize,
    decks: Vec<Deck>,
    strips: Vec<Strip>,
    fx: Vec<FxUnit>,
    fx_state: [FxState; FX_UNITS],
    sinc: SincTable,
    settings: Settings,
    quantize: bool,
    snap: bool,
    limiter_on: bool,
    limiter: PeakLimiter,
    soft_clip: SoftClip,
    master_meter: PeakMeter,
    crossfader: f32,
    crossfader_curve: f32,
    crossfader_reverse: bool,
    main_level: f32,
    cue_mix: f32,
    cue_volume: f32,
    /// FILTER ROLL held, per deck.
    filter_roll: [bool; MAX_DECKS],
    clock: Clock,
    drums: DrumMachine,
    commands: rtrb::Consumer<Command>,
    events: rtrb::Producer<Event>,
    garbage: rtrb::Producer<Garbage>,
    snapshot: triple_buffer::Input<Snapshot>,
    master: Vec<[f32; 2]>,
    cue: Vec<[f32; 2]>,
    bus: [Vec<[f32; 2]>; FX_UNITS],
    frames: u64,
    started: Instant,
    cpu_load: f32,
    recorder: Option<Box<Recorder>>,
}

pub(crate) struct Channels {
    pub commands: rtrb::Consumer<Command>,
    pub events: rtrb::Producer<Event>,
    pub garbage: rtrb::Producer<Garbage>,
    pub snapshot: triple_buffer::Input<Snapshot>,
}

impl Engine {
    pub(crate) fn new(sample_rate: u32, max_block: usize, ch: Channels) -> Self {
        let sr = f64::from(sample_rate);
        let srf = sample_rate as f32;
        let mut fx: Vec<FxUnit> = (0..FX_UNITS).map(|_| FxUnit::new(srf)).collect();
        let mut fx_state = [FxState::default(); FX_UNITS];
        // Default effects: Delay / Reverb / Filter, and Flanger / Gater / Beatmasher.
        for (u, unit) in fx.iter_mut().enumerate() {
            for slot in 0..3 {
                let e = 1 + u * 3 + slot;
                unit.select_effect(slot, e);
                fx_state[u].effects[slot] = e;
            }
            unit.set_dry_wet(0.5);
            fx_state[u].dry_wet = 0.5;
            fx_state[u].knobs = [[0.5; 3]; 3];
            fx_state[u].amount = [rille_dsp::fx::DEFAULT_AMOUNT; 3];
            for slot in 0..3 {
                for k in 0..3 {
                    unit.set_knob(slot, k, 0.5);
                }
            }
        }
        Self {
            sr,
            max_block,
            decks: (0..MAX_DECKS).map(|i| Deck::new(i as u8, sample_rate, max_block)).collect(),
            strips: (0..MAX_DECKS).map(|_| Strip::new(srf)).collect(),
            fx,
            fx_state,
            sinc: SincTable::new(32, 256),
            settings: Settings::default(),
            quantize: true,
            snap: true,
            limiter_on: true,
            limiter: PeakLimiter::new(srf),
            soft_clip: SoftClip::new(-0.3),
            master_meter: PeakMeter::new(srf),
            crossfader: 0.5,
            crossfader_curve: 0.5,
            crossfader_reverse: false,
            main_level: 0.8,
            cue_mix: 0.5,
            cue_volume: 0.8,
            filter_roll: [false; MAX_DECKS],
            clock: Clock {
                master: None,
                auto: true,
                explicit: false,
                bpm: 120.0,
                beat: 0.0,
                bar_origin: 0.0,
                origin_leader: None,
                origin_shift: None,
            },
            drums: DrumMachine::new(sample_rate, max_block),
            commands: ch.commands,
            events: ch.events,
            garbage: ch.garbage,
            snapshot: ch.snapshot,
            master: vec![[0.0; 2]; max_block],
            cue: vec![[0.0; 2]; max_block],
            bus: std::array::from_fn(|_| vec![[0.0; 2]; max_block]),
            frames: 0,
            started: Instant::now(),
            cpu_load: 0.0,
            recorder: None,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sr as u32
    }

    fn modes(&self) -> Modes {
        Modes { quantize: self.quantize, snap: self.snap }
    }

    /// Fills an interleaved device buffer with `channels` channels: master on
    /// 1/2, headphones on 3/4 (or split across 1/2).
    pub fn process_interleaved(&mut self, out: &mut [f32], channels: usize) {
        flush_denormals();
        let channels = channels.max(1);
        let total = out.len() / channels;
        let mut done = 0;
        while done < total {
            let n = (total - done).min(self.max_block);
            self.render(n);
            if let Some(outputs) = self.settings.external_outputs {
                self.write_external(&mut out[done * channels..(done + n) * channels], channels, &outputs);
                done += n;
                continue;
            }
            for i in 0..n {
                let frame = &mut out[(done + i) * channels..(done + i + 1) * channels];
                let [ml, mr] = self.master[i];
                let [cl, cr] = self.cue[i];
                frame.fill(0.0);
                if channels == 1 {
                    frame[0] = 0.5 * (ml + mr);
                } else if self.settings.split_cue && channels < 4 {
                    frame[0] = 0.5 * (cl + cr);
                    frame[1] = 0.5 * (ml + mr);
                } else {
                    frame[0] = ml;
                    frame[1] = mr;
                    if channels >= 4 && self.settings.cue_on_outputs_3_4 {
                        frame[2] = cl;
                        frame[3] = cr;
                    }
                }
            }
            done += n;
        }
    }

    /// External mixer mode: each deck's last rendered block on its own
    /// stereo pair; pairs past the device's channels are dropped.
    fn write_external(&self, out: &mut [f32], channels: usize, outputs: &[Option<u8>; MAX_DECKS]) {
        out.fill(0.0);
        for (deck, pair) in self.decks.iter().zip(outputs) {
            let Some(l) = pair.map(|p| usize::from(p) * 2).filter(|l| l + 1 < channels) else { continue };
            for (frame, s) in out.chunks_exact_mut(channels).zip(deck.buf.iter()) {
                frame[l] += s[0];
                frame[l + 1] += s[1];
            }
        }
    }

    /// Renders one block (at most `max_block` frames) into the master and
    /// headphone buffers; returns them (for offline rendering and tests).
    pub fn render(&mut self, n: usize) -> (&[[f32; 2]], &[[f32; 2]]) {
        let n = n.min(self.max_block);
        let t0 = Instant::now();
        self.drain_commands();
        self.render_decks(n);
        self.mix(n);
        if let Some(r) = self.recorder.as_mut() {
            r.push(&self.master[..n]);
        }
        self.frames += n as u64;
        let block_secs = n as f64 / self.sr;
        let load = (t0.elapsed().as_secs_f64() / block_secs) as f32;
        self.cpu_load += 0.05 * (load - self.cpu_load);
        self.publish();
        (&self.master[..n], &self.cue[..n])
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                Command::Control(ev) => self.control(ev),
                Command::Load { deck, track } => {
                    let d = usize::from(deck).min(MAX_DECKS - 1);
                    let mut garbage = |g| drop(self.garbage.push(g));
                    self.decks[d].load(track, &mut garbage);
                    if self.clock.master == Some(deck) {
                        self.clock.master = None;
                    }
                }
                Command::Unload { deck } if self.decks[usize::from(deck).min(MAX_DECKS - 1)].remix.is_some() => {}
                Command::Unload { deck } => {
                    let d = usize::from(deck).min(MAX_DECKS - 1);
                    let mut garbage = |g| drop(self.garbage.push(g));
                    self.decks[d].unload(&mut garbage);
                    if self.clock.master == Some(deck) {
                        self.clock.master = None;
                    }
                }
                Command::SetGrid { deck, track_id, grid } => {
                    let d = usize::from(deck).min(MAX_DECKS - 1);
                    if self.decks[d].track_id == track_id {
                        let mut garbage = |g| drop(self.garbage.push(g));
                        self.decks[d].set_grid(grid, &mut garbage);
                    } else if let Some(g) = grid {
                        let _ = self.garbage.push(Garbage::Grid(g));
                    }
                }
                Command::SeekSecs { deck, secs } => {
                    let d = usize::from(deck).min(MAX_DECKS - 1);
                    let ctx = RenderCtx { sr_out: self.sr, sinc: &self.sinc, declick: (0.003 * self.sr) as usize };
                    self.decks[d].jump(secs.max(0.0), &ctx);
                }
                Command::Settings(s) => {
                    self.settings = s;
                    self.limiter.set_ceiling_db(s.limiter_ceiling_db);
                }
                Command::SetClockBpm(bpm) => self.set_clock_bpm(bpm),
                Command::SetMaster(m) => self.set_master(m),
                Command::SetRemix { deck, remix } => {
                    let d = usize::from(deck).min(MAX_DECKS - 1);
                    let mut garbage = |g| drop(self.garbage.push(g));
                    self.decks[d].set_remix(remix, &mut garbage);
                    if self.clock.master == Some(deck) {
                        self.clock.master = None;
                    }
                }
                Command::SetRemixCell { deck, cell, sample } => {
                    let d = usize::from(deck).min(MAX_DECKS - 1);
                    let slot = self.decks[d].remix.as_mut().and_then(|r| r.cells.get_mut(usize::from(cell)));
                    let old = match slot {
                        Some(c) => std::mem::replace(c, sample),
                        None => sample,
                    };
                    if let Some(s) = old {
                        let _ = self.garbage.push(Garbage::Sample(s));
                    }
                }
                Command::SetStems { deck, track_id, stems } => {
                    let d = usize::from(deck).min(MAX_DECKS - 1);
                    let mut garbage = |g| drop(self.garbage.push(g));
                    self.decks[d].set_stems(track_id, stems, &mut garbage);
                }
                Command::ExtendTrack { deck, track_id, audio, length_secs } => {
                    let d = usize::from(deck).min(MAX_DECKS - 1);
                    let mut garbage = |g| drop(self.garbage.push(g));
                    self.decks[d].extend(track_id, audio, length_secs, &mut garbage);
                }
                Command::Record(recorder) => {
                    if let Some(old) = std::mem::replace(&mut self.recorder, recorder) {
                        let _ = self.garbage.push(Garbage::Recorder(old));
                    }
                }
                Command::SetDrumKit(kit) => {
                    for old in self.drums.set_kit(kit).into_iter().flatten() {
                        let _ = self.garbage.push(Garbage::DrumKit(old));
                    }
                }
                Command::SetDrumPattern { index, pattern, undo } => {
                    self.drums.set_pattern(usize::from(index), pattern, undo)
                }
                Command::SetDrumParams(p) => self.drums.set_params(p),
            }
        }
    }

    /// Sets the master tempo. A deck that leads moves its tempo fader to
    /// play at it, as far as the tempo range allows.
    fn set_clock_bpm(&mut self, bpm: f64) {
        self.clock.bpm = bpm.clamp(40.0, 250.0);
        let range = self.settings.tempo_range.max(1e-6);
        if let Some(m) = self.clock.master {
            let deck = &mut self.decks[usize::from(m)];
            if let Some(own) = deck.track_bpm() {
                let rate = self.clock.bpm / own;
                deck.tempo_fader = (0.5 + (rate - 1.0) / (2.0 * range)).clamp(0.0, 1.0) as f32;
            }
        }
    }

    fn set_master(&mut self, m: Option<u8>) {
        match m {
            Some(d) if usize::from(d) < MAX_DECKS => {
                self.clock.master = Some(d);
                self.clock.auto = true;
                self.clock.explicit = true;
                if let Some(b) = self.decks[usize::from(d)].beat() {
                    self.clock.beat = b;
                }
            }
            _ => {
                // Internal clock leads.
                self.clock.master = None;
                self.clock.auto = false;
                self.clock.explicit = false;
            }
        }
    }

    /// Starts, re-cuts or ends deck `unit`'s FILTER ROLL from its filter knob.
    fn update_roll(&mut self, unit: usize) {
        let beats = if self.filter_roll[unit] { filter_roll_beats(self.strips[unit].filter) } else { None };
        let ctx = RenderCtx {
            sr_out: self.sr,
            sinc: &self.sinc,
            declick: (f64::from(self.settings.declick_ms) * 0.001 * self.sr) as usize,
        };
        self.decks[unit].roll(beats, &ctx);
    }

    fn control(&mut self, ev: ControlEvent) {
        let ControlEvent { target, value } = ev;
        let unit = usize::from(target.unit);
        let press = value.is_press();
        let held = match value {
            ControlValue::Press(down) => Some(down),
            _ => None,
        };
        let abs = match value {
            ControlValue::Absolute(v) => Some(v.clamp(0.0, 1.0)),
            _ => None,
        };
        match target.control.scope() {
            Scope::Global => match target.control {
                Control::Crossfader => self.crossfader = abs.unwrap_or(self.crossfader),
                Control::CrossfaderCurve => self.crossfader_curve = abs.unwrap_or(self.crossfader_curve),
                Control::CrossfaderReverse => self.crossfader_reverse = held.unwrap_or(self.crossfader_reverse),
                Control::MainLevel => self.main_level = abs.unwrap_or(self.main_level),
                Control::CueMix => self.cue_mix = abs.unwrap_or(self.cue_mix),
                Control::CueVolume => self.cue_volume = abs.unwrap_or(self.cue_volume),
                Control::Quantize if press => self.quantize = !self.quantize,
                Control::Snap if press => self.snap = !self.snap,
                Control::Limiter if press => self.limiter_on = !self.limiter_on,
                Control::ClockTempo => match value {
                    ControlValue::Absolute(v) => self.clock.bpm = 60.0 + f64::from(v.clamp(0.0, 1.0)) * 120.0,
                    ControlValue::Delta(d) => {
                        self.clock.bpm = (self.clock.bpm + f64::from(d) * 0.01).clamp(40.0, 250.0)
                    }
                    _ => {}
                },
                _ => {}
            },
            Scope::Fx if unit < FX_UNITS => self.fx_control(unit, target.control, value),
            Scope::Drum => {
                self.drums.control(target.control, value);
            }
            Scope::Deck if unit < MAX_DECKS => {
                let strip = &mut self.strips[unit];
                match target.control {
                    Control::Gain => strip.gain = abs.unwrap_or(strip.gain),
                    Control::EqHi => strip.eq[2] = abs.unwrap_or(strip.eq[2]),
                    Control::EqMid => strip.eq[1] = abs.unwrap_or(strip.eq[1]),
                    Control::EqLo => strip.eq[0] = abs.unwrap_or(strip.eq[0]),
                    Control::EqLoKill => strip.kill[0] = held.unwrap_or(strip.kill[0]),
                    Control::EqMidKill => strip.kill[1] = held.unwrap_or(strip.kill[1]),
                    Control::EqHiKill => strip.kill[2] = held.unwrap_or(strip.kill[2]),
                    Control::Filter => {
                        strip.filter = abs.unwrap_or(strip.filter);
                        if self.filter_roll[unit] {
                            self.update_roll(unit);
                        }
                    }
                    Control::FilterRoll => {
                        if let Some(down) = held {
                            self.filter_roll[unit] = down;
                            self.update_roll(unit);
                        }
                    }
                    Control::Volume => strip.volume = abs.unwrap_or(strip.volume),
                    Control::Pfl if press => strip.pfl = !strip.pfl,
                    Control::FxAssign(n) if press && (1..=FX_UNITS as u8).contains(&n) => {
                        let i = usize::from(n - 1);
                        strip.fx_assign[i] = !strip.fx_assign[i];
                    }
                    Control::Sync if press => self.toggle_sync(unit),
                    Control::Tempo if self.decks[unit].sync && self.clock.master != Some(unit as u8) => {
                        if let ControlValue::Absolute(v) = value {
                            self.synced_tempo(unit, v.clamp(0.0, 1.0));
                        }
                    }
                    Control::Master if press => {
                        let m = if self.clock.master == Some(unit as u8) { None } else { Some(unit as u8) };
                        if m.is_none() {
                            self.clock.master = None;
                            self.clock.auto = true;
                            self.clock.explicit = false;
                        } else {
                            self.set_master(m);
                        }
                    }
                    c => {
                        let modes = self.modes();
                        let ctx = RenderCtx {
                            sr_out: self.sr,
                            sinc: &self.sinc,
                            declick: (f64::from(self.settings.declick_ms) * 0.001 * self.sr) as usize,
                        };
                        if let Some(e) = self.decks[unit].control(c, value, &modes, &ctx) {
                            let _ = self.events.push(e);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn fx_control(&mut self, u: usize, c: Control, value: ControlValue) {
        let (unit, st) = (&mut self.fx[u], &mut self.fx_state[u]);
        match (c, value) {
            (Control::FxOn, ControlValue::Press(true)) => {
                st.on = !st.on;
                unit.set_on(st.on);
            }
            (Control::FxDryWet, ControlValue::Absolute(v)) => {
                st.dry_wet = v.clamp(0.0, 1.0);
                unit.set_dry_wet(st.dry_wet);
            }
            // Group mode: knob n is slot n's amount, button n switches it,
            // and the parameter control reaches its first parameter.
            (Control::FxKnob(n), ControlValue::Absolute(v)) if (1..=3).contains(&n) => {
                let slot = usize::from(n - 1);
                st.amount[slot] = v.clamp(0.0, 1.0);
                unit.set_amount(slot, st.amount[slot]);
            }
            (Control::FxParam(n), ControlValue::Absolute(v)) if (1..=3).contains(&n) => {
                let slot = usize::from(n - 1);
                st.knobs[slot][0] = v.clamp(0.0, 1.0);
                unit.set_knob(slot, 0, st.knobs[slot][0]);
            }
            (Control::FxButton(n), ControlValue::Press(true)) if (1..=3).contains(&n) => {
                let slot = usize::from(n - 1);
                st.enabled[slot] = !st.enabled[slot];
                unit.set_slot_on(slot, st.enabled[slot]);
            }
            (Control::FxSelect(n), ControlValue::Delta(d)) if (1..=3).contains(&n) => {
                let slot = usize::from(n - 1);
                let count = FxUnit::effect_names().len() as i64;
                let e = (st.effects[slot] as i64 + d.signum() as i64).rem_euclid(count) as usize;
                st.effects[slot] = e;
                unit.select_effect(slot, e);
            }
            _ => {}
        }
    }

    fn toggle_sync(&mut self, d: usize) {
        let deck = &mut self.decks[d];
        deck.sync = !deck.sync;
        if !deck.sync {
            return;
        }
        deck.phase_offset = 0.0;
        deck.jog_offset = 0.0;
        deck.nudging = false;
        // Follow at the tempo octave closest to the track's own tempo.
        if let Some(own) = deck.track_bpm() {
            let lead = self.clock.bpm;
            deck.sync_mult = [0.5, 1.0, 2.0]
                .into_iter()
                .min_by(|a, b| ((a * lead / own).ln().abs()).total_cmp(&(b * lead / own).ln().abs()))
                .unwrap_or(1.0);
        }
        deck.just_started = deck.playing;
    }

    /// A synced deck that does not lead follows the clock, so its tempo
    /// fader moves the tempo everyone syncs to: the leader's, or the clock's
    /// while no deck leads.
    fn synced_tempo(&mut self, unit: usize, fader: f32) {
        let range = self.settings.tempo_range.max(1e-6);
        let deck = &mut self.decks[unit];
        deck.tempo_fader = fader;
        let Some(own) = deck.track_bpm() else { return };
        let rate = 1.0 + (f64::from(fader) - 0.5) * 2.0 * range;
        let bpm = (own * rate / deck.sync_mult).clamp(40.0, 250.0);
        let leader = self.clock.master.map(usize::from).filter(|&m| self.decks[m].playing);
        match leader.and_then(|m| Some((m, self.decks[m].track_bpm()?))) {
            Some((m, lead)) => {
                self.decks[m].tempo_fader = (0.5 + (bpm / lead - 1.0) / (2.0 * range)).clamp(0.0, 1.0) as f32;
            }
            None => self.clock.bpm = bpm,
        }
    }

    /// The deck that leads, choosing one automatically when needed.
    fn leader(&mut self) -> Option<usize> {
        let unsynced_ready =
            (0..MAX_DECKS).find(|&i| self.decks[i].playing && self.decks[i].grid.is_some() && !self.decks[i].sync);
        if let Some(m) = self.clock.master {
            let d = &self.decks[usize::from(m)];
            if d.playing && d.grid.is_some() {
                // A synced deck only leads while no unsynced deck can (e.g.
                // it was still being analyzed); then it hands over.
                if self.clock.auto
                    && !self.clock.explicit
                    && d.sync
                    && let Some(i) = unsynced_ready
                {
                    self.clock.master = Some(i as u8);
                    if let Some(b) = self.decks[i].beat() {
                        self.clock.beat = b;
                    }
                    self.realign_followers(i);
                    return Some(i);
                }
                return Some(usize::from(m));
            }
            if self.clock.auto {
                self.clock.master = None;
                self.clock.explicit = false;
            }
        }
        if self.clock.auto && self.clock.master.is_none() {
            // First playing deck with a grid takes over; unsynced decks first
            // (synced ones would rather follow).
            let find = |synced: bool| {
                (0..MAX_DECKS)
                    .find(|&i| self.decks[i].playing && self.decks[i].grid.is_some() && self.decks[i].sync == synced)
            };
            let pick = find(false).or_else(|| find(true));
            if let Some(i) = pick {
                let deck = &mut self.decks[i];
                if deck.sync
                    && let Some(own) = deck.track_bpm()
                {
                    // Keep the current tempo when a synced deck takes the lead.
                    let rate = self.clock.bpm * deck.sync_mult / own;
                    let range = self.settings.tempo_range.max(1e-6);
                    deck.tempo_fader = (0.5 + (rate - 1.0) / (2.0 * range)).clamp(0.0, 1.0) as f32;
                }
                self.clock.master = Some(i as u8);
                if let Some(b) = self.decks[i].beat() {
                    self.clock.beat = b / self.decks[i].sync_mult;
                }
                self.realign_followers(i);
                return Some(i);
            }
        }
        None
    }

    /// After the leader changes, synced decks land in phase with it at once
    /// (crossfaded jump) instead of pulling their tempo for seconds.
    fn realign_followers(&mut self, leader: usize) {
        for (j, d) in self.decks.iter_mut().enumerate() {
            if j != leader && d.sync && d.playing {
                d.just_started = true;
            }
        }
    }

    fn render_decks(&mut self, n: usize) {
        let block_secs = n as f64 / self.sr;
        let leader = self.leader();
        let range = self.settings.tempo_range;
        let ctx = RenderCtx {
            sr_out: self.sr,
            sinc: &self.sinc,
            declick: (f64::from(self.settings.declick_ms) * 0.001 * self.sr) as usize,
        };

        // 1. The leader plays at its own tempo; the clock follows it.
        let advance = if let Some(m) = leader {
            let deck = &mut self.decks[m];
            let speed = deck.free_speed(n, self.sr, range);
            let (beats, ev) = deck.render(n, speed, true, &ctx);
            if let Some(e) = ev {
                let _ = self.events.push(e);
            }
            if let Some(bpm) = deck.track_bpm()
                && speed > 0.0
            {
                self.clock.bpm = bpm * speed;
            }
            // Travel for the followers' speed: continuous motion only.
            let expected = self.clock.bpm / 60.0 * block_secs;
            let travel = if (beats - expected).abs() > 0.25 { expected } else { beats };
            self.clock.beat += travel;
            // The phase: the leader's audible beat, unless it is scratched,
            // reversed or looping inside a beat (then the clock runs on).
            let short_loop = deck.loop_active
                && deck
                    .loop_range
                    .is_some_and(|(s, e)| deck.grid.as_ref().is_some_and(|g| g.beat_at(e) - g.beat_at(s) < 0.99));
            if !deck.reverse
                && !deck.is_scratching()
                && !short_loop
                && let Some(b) = deck.beat()
            {
                let heard = b / deck.sync_mult;
                self.clock.beat = heard + (self.clock.beat - heard).round();
                if let Some(grid) = deck.grid.as_deref() {
                    let k = deck.sync_mult;
                    let bar_len = f64::from(grid.beats_per_bar.max(1)) / k;
                    let cand = self.clock.beat - grid.bar_phase(b) / k;
                    let force = self.clock.origin_leader != Some(m as u8) || self.drums.origin_request;
                    let rolling = deck.loop_active || deck.flux;
                    update_origin(&mut self.clock, cand, bar_len, force, rolling);
                    self.clock.origin_leader = Some(m as u8);
                }
            }
            travel
        } else {
            let travel = self.clock.bpm / 60.0 * block_secs;
            self.clock.beat += travel;
            self.clock.origin_leader = None;
            self.clock.origin_shift = None;
            travel
        };
        let clock_end = self.clock.beat;

        // 2. Everyone else: synced decks lock to the clock.
        for i in 0..MAX_DECKS {
            if Some(i) == leader {
                self.decks[i].just_started = false;
                continue;
            }
            let deck = &mut self.decks[i];
            deck.since_realign += block_secs;
            let lockable = deck.sync && deck.playing && !deck.reverse && !deck.is_scratching();
            let speed = match (lockable, deck.grid.clone()) {
                (true, Some(grid)) => {
                    let k = deck.sync_mult;
                    // The tempo fader follows, so switching sync off keeps the tempo.
                    let own = grid.bpm_at(deck.position());
                    if own > 0.0 {
                        let rate = self.clock.bpm * k / own;
                        deck.tempo_fader = (0.5 + (rate - 1.0) / (2.0 * range.max(1e-6))).clamp(0.0, 1.0) as f32;
                    }
                    // Bend buttons push a synced deck out of phase only while
                    // held; afterwards it glides back.
                    if deck.bend != 0.0 {
                        deck.phase_offset += deck.bend * 0.03 * k * advance;
                    } else if deck.phase_offset != 0.0 {
                        let step = (MAX_PHASE_PULL * k * advance).abs();
                        deck.phase_offset -= deck.phase_offset.clamp(-step, step);
                    }
                    let b0 = grid.beat_at(deck.position());
                    // Error at the end of this block if it just keeps pace.
                    let target_phase = (k * clock_end + deck.phase_offset + deck.jog_offset).rem_euclid(1.0);
                    let err = wrap_phase_error(target_phase - (b0 + k * advance).rem_euclid(1.0));
                    let fractional_loop = deck.loop_active
                        && deck.loop_range.is_some_and(|(s, e)| {
                            let beats = grid.beat_at(e) - grid.beat_at(s);
                            (beats - beats.round()).abs() > 0.01
                        });
                    let audible = deck.speed.abs() > 0.0;
                    // A jump would land on a running crossfade (a loop wrap
                    // in this block, or the last jump still fading): wait
                    // for the next block instead.
                    let jump_pending = deck.fading()
                        || deck.loop_active
                            && deck.loop_range.is_some_and(|(_, e)| grid.secs_at(b0 + err.max(0.0) + k * advance) >= e);
                    // A nudge moves the deck on purpose: no jump into phase.
                    let realign = !fractional_loop
                        && !(audible && jump_pending)
                        && !deck.nudging
                        && err.abs() >= REALIGN_BEATS
                        && (deck.just_started || deck.since_realign >= REALIGN_INTERVAL_SECS);
                    if (deck.just_started && !audible) || realign {
                        // Land in phase at once: silently when just starting,
                        // otherwise with a crossfade.
                        let aligned = grid.secs_at(b0 + err);
                        let (a0, a1) = (b0 + err, b0 + err + k * advance);
                        let speed = (grid.secs_at(a1) - grid.secs_at(a0)) / block_secs;
                        // The new voice is primed for the speed it will play
                        // at, not the last block's (e.g. a scratch).
                        deck.speed = speed;
                        if audible {
                            deck.jump(aligned, &ctx)
                        } else {
                            deck.set_position(aligned)
                        }
                        deck.since_realign = 0.0;
                        if audible {
                            deck.realigns = deck.realigns.wrapping_add(1);
                        }
                        speed
                    } else {
                        // A nudge may pull harder (up to 30 % faster or
                        // slower) than drift corrections; the gain stays the
                        // same because a keylocked deck is heard only after
                        // the stretcher's latency, and a larger gain would
                        // overshoot.
                        let gain = (block_secs / PHASE_CORRECTION_SECS).min(1.0);
                        let pull = if deck.nudging { NUDGE_PULL } else { MAX_PHASE_PULL };
                        let limit = (pull * k * advance).abs();
                        let corr = if fractional_loop { 0.0 } else { (err * gain).clamp(-limit, limit) };
                        if deck.nudging && err.abs() < 2e-4 {
                            deck.nudging = false;
                        }
                        let b1 = b0 + k * advance + corr;
                        (grid.secs_at(b1) - grid.secs_at(b0)) / block_secs
                    }
                }
                _ => {
                    // Unsynced (or synced without a grid): own tempo; a
                    // synced deck without a lead still takes the clock tempo.
                    let mut s = deck.free_speed(n, self.sr, range);
                    if deck.sync
                        && deck.playing
                        && !deck.is_scratching()
                        && let Some(own) = deck.track_bpm()
                    {
                        s = s.signum() * self.clock.bpm * deck.sync_mult / own;
                    }
                    s
                }
            };
            deck.just_started = false;
            let (_, ev) = deck.render(n, speed, true, &ctx);
            if let Some(e) = ev {
                let _ = self.events.push(e);
            }
        }

        self.drums.render(n, clock_end, self.clock.bar_origin, self.clock.bpm);
        if let Some(old) = self.drums.take_faded_kit() {
            let _ = self.garbage.push(Garbage::DrumKit(old));
        }
    }

    fn mix(&mut self, n: usize) {
        self.master[..n].fill([0.0; 2]);
        self.cue[..n].fill([0.0; 2]);
        for b in &mut self.bus {
            b[..n].fill([0.0; 2]);
        }
        if self.settings.external_outputs.is_some() {
            self.mix_external(n);
            return;
        }
        let chain = (0..MAX_DECKS).any(|i| self.strips[i].fx_assign.iter().all(|a| *a))
            || self.drums.strip.fx_assign.iter().all(|a| *a);
        for i in 0..MAX_DECKS {
            let deck = &mut self.decks[i];
            let strip = &mut self.strips[i];
            let buf = &mut deck.buf[..n];
            strip.process_pre(buf, deck.auto_gain_db);
            if strip.pfl {
                for (c, s) in self.cue[..n].iter_mut().zip(buf.iter()) {
                    c[0] += s[0];
                    c[1] += s[1];
                }
            }
            let x = if self.crossfader_reverse { 1.0 - self.crossfader } else { self.crossfader };
            strip.process_post(buf, crossfader_gain(x, i % 2, self.crossfader_curve));
            let dest = match strip.fx_assign {
                [true, _] => &mut self.bus[0],
                [false, true] => &mut self.bus[1],
                _ => &mut self.master,
            };
            for (d, s) in dest[..n].iter_mut().zip(buf.iter()) {
                d[0] += s[0];
                d[1] += s[1];
            }
        }
        // The drum machine's own channel: not on the crossfader.
        {
            let (strip, buf) = (&mut self.drums.strip, &mut self.drums.buf[..n]);
            strip.process_pre(buf, 0.0);
            if strip.pfl {
                for (c, s) in self.cue[..n].iter_mut().zip(buf.iter()) {
                    c[0] += s[0];
                    c[1] += s[1];
                }
            }
            strip.process_post(buf, 1.0);
            let dest = match strip.fx_assign {
                [true, _] => &mut self.bus[0],
                [false, true] => &mut self.bus[1],
                _ => &mut self.master,
            };
            for (d, s) in dest[..n].iter_mut().zip(buf.iter()) {
                d[0] += s[0];
                d[1] += s[1];
            }
        }

        let bpm = self.clock.bpm.max(1.0);
        let ctx = FxCtx {
            sample_rate: self.sr as f32,
            bpm,
            beat_pos: self.clock.beat - bpm / 60.0 * n as f64 / self.sr,
            beats_per_bar: 4,
        };
        let [bus0, bus1] = &mut self.bus;
        self.fx[0].process(&mut bus0[..n], &ctx);
        if chain {
            for (d, s) in bus1[..n].iter_mut().zip(bus0[..n].iter()) {
                d[0] += s[0];
                d[1] += s[1];
            }
        } else {
            for (d, s) in self.master[..n].iter_mut().zip(bus0[..n].iter()) {
                d[0] += s[0];
                d[1] += s[1];
            }
        }
        self.fx[1].process(&mut bus1[..n], &ctx);
        for (d, s) in self.master[..n].iter_mut().zip(bus1[..n].iter()) {
            d[0] += s[0];
            d[1] += s[1];
        }

        let level = (self.main_level / 0.8).powi(2);
        for f in &mut self.master[..n] {
            f[0] *= level;
            f[1] *= level;
        }
        if self.limiter_on {
            self.limiter.process(&mut self.master[..n]);
        } else {
            self.soft_clip.process(&mut self.master[..n]);
        }
        self.master_meter.process(&self.master[..n]);

        let cue_level = (self.cue_volume / 0.8).powi(2);
        let (wet, dry) = (self.cue_mix, 1.0 - self.cue_mix);
        for (c, m) in self.cue[..n].iter_mut().zip(self.master[..n].iter()) {
            c[0] = (c[0] * dry + m[0] * wet) * cue_level;
            c[1] = (c[1] * dry + m[1] * wet) * cue_level;
        }
    }

    /// External mixer mode: GAIN, EQ and filter on every deck, then the FX
    /// units as inserts. A unit holds one signal's state (delay lines,
    /// reverb tails), so it runs on the first deck assigned to it; a deck on
    /// both units goes through 1, then 2. The decks stay in their buffers
    /// for [`Self::write_external`]; there is no main or headphone mix.
    fn mix_external(&mut self, n: usize) {
        for (deck, strip) in self.decks.iter_mut().zip(self.strips.iter_mut()) {
            strip.process_pre(&mut deck.buf[..n], deck.auto_gain_db);
        }
        let bpm = self.clock.bpm.max(1.0);
        let ctx = FxCtx {
            sample_rate: self.sr as f32,
            bpm,
            beat_pos: self.clock.beat - bpm / 60.0 * n as f64 / self.sr,
            beats_per_bar: 4,
        };
        for (u, fx) in self.fx.iter_mut().enumerate() {
            if let Some(d) = self.strips.iter().position(|s| s.fx_assign[u]) {
                fx.process(&mut self.decks[d].buf[..n], &ctx);
            }
        }
        self.master_meter.process(&self.master[..n]);
    }

    fn publish(&mut self) {
        let clock = &self.clock;
        let mut snap = Snapshot {
            crossfader: self.crossfader,
            crossfader_curve: self.crossfader_curve,
            crossfader_reverse: self.crossfader_reverse,
            main_level: self.main_level,
            cue_mix: self.cue_mix,
            cue_volume: self.cue_volume,
            quantize: self.quantize,
            snap: self.snap,
            limiter: self.limiter_on,
            master_meter: self.master_meter.peak(),
            limiter_reduction_db: self.limiter.gain_reduction_db(),
            master_deck: clock.master,
            clock_bpm: clock.bpm,
            clock_beat: clock.beat,
            sample_rate: self.sr as u32,
            frames: self.frames,
            time_nanos: self.started.elapsed().as_nanos() as u64,
            cpu_load: self.cpu_load,
            fx: self.fx_state,
            drums: self.drums.state(clock.beat, clock.bar_origin),
            ..Snapshot::default()
        };
        for (i, u) in snap.fx.iter_mut().enumerate() {
            u.on = self.fx[i].is_on();
        }
        for i in 0..MAX_DECKS {
            let d = &self.decks[i];
            let s = &self.strips[i];
            let pos = d.position();
            let beat = d.beat().unwrap_or(0.0);
            let track_bpm = d.track_bpm().unwrap_or(0.0);
            let (ls, le) = d.loop_range.unwrap_or((0.0, 0.0));
            let phase_error = if d.grid.is_some() {
                wrap_phase_error((d.sync_mult * clock.beat).rem_euclid(1.0) - beat.rem_euclid(1.0))
            } else {
                0.0
            };
            // Synced decks show the tempo they follow, not the momentary
            // speed that includes phase correction.
            let synced = d.sync && d.playing && track_bpm > 0.0 && clock.master != Some(i as u8);
            let rate = if synced {
                clock.bpm * d.sync_mult / track_bpm
            } else if d.playing {
                d.speed.abs()
            } else {
                d.own_rate(self.settings.tempo_range)
            };
            if let Some(r) = &d.remix {
                snap.remix[i] = r.state(beat);
            }
            snap.decks[i] = DeckState {
                loaded: d.track.is_some() || d.remix.is_some(),
                remix: d.remix.is_some(),
                track_id: d.track_id,
                duration_secs: d.duration(),
                position_secs: pos,
                playing: d.playing,
                sync: d.sync,
                master: clock.master == Some(i as u8),
                keylock: d.keylock,
                tick: d.tick,
                realigns: d.realigns,
                flux: d.flux,
                reverse: d.reverse,
                rate,
                tempo_fader: d.tempo_fader,
                track_bpm,
                bpm: track_bpm * rate,
                key_shift: d.key_shift,
                main_cue_secs: d.main_cue,
                hotcues: d.hotcues,
                loop_start_secs: ls,
                loop_end_secs: le,
                loop_set: d.loop_range.is_some(),
                loop_active: d.loop_active,
                loop_size_idx: d.loop_size.min(LOOP_SIZES.len() - 1),
                beat,
                phase_error,
                speed: d.speed,
                cue_held: d.cue_held(),
                end_warning: d.playing && d.track.is_some() && d.duration() - pos < 30.0,
                arrived_secs: d.arrived_secs(),
                buffering: d.buffering,
                stems: d.stems.is_some(),
                stem_volume: d.stem_volume,
                stem_mute: d.stem_mute,
            };
            snap.channels[i] = ChannelState {
                gain: s.gain,
                eq: s.eq,
                kill: s.kill,
                filter: s.filter,
                volume: s.volume,
                pfl: s.pfl,
                fx_assign: s.fx_assign,
                meter: s.meter.peak(),
            };
        }
        self.snapshot.write(snap);
    }
}

/// Takes the bar origin `cand` (a clock beat on the leader's downbeat; bars
/// are `bar_len` clock beats) on the lattice nearest the current origin. A
/// different lattice is taken at once when `force`d, otherwise only when it
/// has held for a beat and the leader is not looping or rolling (a loop
/// wrap or a roll moves the leader's beats only for a moment).
fn update_origin(clock: &mut Clock, cand: f64, bar_len: f64, force: bool, rolling: bool) {
    let snapped = cand + ((clock.bar_origin - cand) / bar_len).round() * bar_len;
    let shift = snapped - clock.bar_origin;
    if force {
        clock.bar_origin = snapped;
        clock.origin_shift = None;
    } else if shift.abs() < 1e-6 || rolling {
        clock.origin_shift = None;
    } else {
        match clock.origin_shift {
            Some((s, since)) if (s - shift).abs() < 1e-3 => {
                if clock.beat - since >= 1.0 {
                    clock.bar_origin = snapped;
                    clock.origin_shift = None;
                }
            }
            _ => clock.origin_shift = Some((shift, clock.beat)),
        }
    }
}
