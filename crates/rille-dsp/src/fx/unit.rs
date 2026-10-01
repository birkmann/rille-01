//! FX unit in group mode: three effect slots in series and a dry/wet knob.
//!
//! As in Traktor's group mode, each slot has an on/off switch and an amount
//! knob, and the track always passes through the chain: tail effects (delay,
//! reverb) add their output on top of it, the others are blended in by their
//! amount.

use super::{EFFECT_NAMES, Effect, FxCtx, new_effect};
use crate::clamp01;
use crate::smooth::LinearSmoother;

/// Effect slots per unit.
pub const SLOTS: usize = 3;
/// Blocks are processed in chunks of at most this many frames.
const MAX_CHUNK: usize = 256;
const SWITCH_MS: f32 = 10.0;
const ON_MS: f32 = 10.0;
const DRY_WET_MS: f32 = 20.0;
const AMOUNT_MS: f32 = 20.0;
/// Amount of a slot before its knob is touched.
pub const DEFAULT_AMOUNT: f32 = 0.5;
/// Wet level below which a switched-off unit counts as silent (−100 dB).
const SILENCE: f32 = 1e-5;
const IDLE_AFTER_SECS: f32 = 0.5;

/// One slot: an instance of every effect type plus the selection.
struct Slot {
    /// `effects[i]` is effect `i + 1` of [`EFFECT_NAMES`].
    effects: Box<[Box<dyn Effect>]>,
    /// Effect being processed (0 = none).
    active: usize,
    /// Effect chosen by the user; differs from `active` while the old one fades out.
    selected: usize,
    /// Fades the slot between bypass (0) and the effect (1).
    mix: LinearSmoother,
    /// Switched on by its button.
    enabled: bool,
    amount: LinearSmoother,
    /// Input level of a tail effect: fades out when the slot is switched
    /// off, so its echoes ring out.
    send: LinearSmoother,
    /// A switched-off tail effect is still audible.
    ringing: bool,
    silent_frames: usize,
}

impl Slot {
    fn new(sample_rate: f32) -> Self {
        Self {
            effects: (1..EFFECT_NAMES.len()).filter_map(|i| new_effect(i, sample_rate)).collect(),
            active: 0,
            selected: 0,
            mix: LinearSmoother::new(sample_rate, SWITCH_MS, 0.0),
            enabled: false,
            amount: LinearSmoother::new(sample_rate, AMOUNT_MS, DEFAULT_AMOUNT),
            send: LinearSmoother::new(sample_rate, ON_MS, 0.0),
            ringing: false,
            silent_frames: 0,
        }
    }

    fn effect(&mut self, idx: usize) -> Option<&mut dyn Effect> {
        Some(self.effects.get_mut(idx.checked_sub(1)?)?.as_mut())
    }

    fn is_bypassed(&self) -> bool {
        self.mix.is_settled() && self.mix.value() == 0.0
    }

    fn target_mix(&self, running: bool) -> f32 {
        match self.active.checked_sub(1).map(|i| &self.effects[i]) {
            Some(fx) if self.selected == self.active && (running || (fx.has_tail() && self.ringing)) => 1.0,
            _ => 0.0,
        }
    }

    /// Switches to the selected effect (resetting it) if the old one is faded out.
    fn complete_switch(&mut self) {
        if self.selected != self.active && self.is_bypassed() {
            self.active = self.selected;
            if let Some(fx) = self.effect(self.active) {
                fx.reset();
            }
        }
    }

    /// `chain` holds the track (and earlier slots) on entry and this slot's
    /// output on return; `dry` is scratch space.
    fn process(&mut self, chain: &mut [[f32; 2]], dry: &mut [[f32; 2]], ctx: &FxCtx, unit_on: bool, idle_after: usize) {
        self.complete_switch();
        let running = self.enabled && unit_on;
        self.ringing |= running;
        self.mix.set_target(self.target_mix(running));
        self.send.set_target(if running { 1.0 } else { 0.0 });
        if self.is_bypassed() {
            return;
        }
        let (send, mix, amount) = (&mut self.send, &mut self.mix, &mut self.amount);
        let Some(fx) = self.effects.get_mut(self.active.wrapping_sub(1)).map(|f| f.as_mut()) else { return };
        let tail = fx.has_tail();
        dry.copy_from_slice(chain);
        if tail {
            for c in chain.iter_mut() {
                let g = send.tick();
                *c = [c[0] * g, c[1] * g];
            }
        }
        fx.process(chain, ctx);
        let mut peak = 0f32;
        for (c, d) in chain.iter_mut().zip(dry.iter()) {
            let g = mix.tick() * amount.tick();
            peak = peak.max(c[0].abs()).max(c[1].abs());
            *c = if tail {
                [d[0] + g * c[0], d[1] + g * c[1]]
            } else {
                [d[0] + g * (c[0] - d[0]), d[1] + g * (c[1] - d[1])]
            };
        }
        // A switched-off tail stops costing CPU once it has died away.
        if tail && !running && send.is_settled() && peak < SILENCE {
            self.silent_frames += chain.len();
            if self.silent_frames >= idle_after {
                self.ringing = false;
                self.mix.set_immediate(0.0);
            }
        } else {
            self.silent_frames = 0;
        }
    }
}

/// FX unit in group mode: the three slots run in series on the wet path,
/// mixed with the dry signal by the (smoothed, linear) dry/wet knob.
///
/// Every slot holds a preallocated instance of every effect, so switching
/// effects never allocates: the old effect fades out, the new one is reset
/// and fades in. When the unit is switched off the effect input fades to
/// silence while delay and reverb tails keep ringing into the output; once
/// the wet signal is silent the unit goes idle and costs nothing.
pub struct FxUnit {
    slots: [Slot; SLOTS],
    on: bool,
    idle: bool,
    input: LinearSmoother,
    dry_wet: LinearSmoother,
    wet: Box<[[f32; 2]]>,
    slot_dry: Box<[[f32; 2]]>,
    in_gain: Box<[f32]>,
    silent_frames: usize,
    idle_after: usize,
}

impl FxUnit {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            slots: std::array::from_fn(|_| Slot::new(sample_rate)),
            on: false,
            idle: true,
            input: LinearSmoother::new(sample_rate, ON_MS, 0.0),
            dry_wet: LinearSmoother::new(sample_rate, DRY_WET_MS, 0.5),
            wet: vec![[0.0; 2]; MAX_CHUNK].into_boxed_slice(),
            slot_dry: vec![[0.0; 2]; MAX_CHUNK].into_boxed_slice(),
            in_gain: vec![0.0; MAX_CHUNK].into_boxed_slice(),
            silent_frames: 0,
            idle_after: (IDLE_AFTER_SECS * sample_rate) as usize,
        }
    }

    /// Effect names by index; 0 is "None".
    pub fn effect_names() -> &'static [&'static str] {
        &EFFECT_NAMES
    }

    pub fn set_on(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        self.on = on;
        self.input.set_target(if on { 1.0 } else { 0.0 });
        let wake = std::mem::replace(&mut self.idle, false) && on;
        self.silent_frames = 0;
        for slot in &mut self.slots {
            let bypassed = slot.is_bypassed();
            let Some(fx) = slot.effect(slot.active) else { continue };
            if !on {
                fx.release();
            } else if wake || (bypassed && !fx.has_tail()) {
                // Fresh start, e.g. the beatmasher re-arms on the next beat.
                fx.reset();
            }
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// True while switched off and silent: `process` leaves the audio untouched.
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// Switches `slot`'s effect on or off (its button in group mode).
    pub fn set_slot_on(&mut self, slot: usize, on: bool) {
        let unit_on = self.on;
        let Some(s) = self.slots.get_mut(slot) else { return };
        if s.enabled == on {
            return;
        }
        s.enabled = on;
        let bypassed = s.is_bypassed();
        let Some(fx) = s.effect(s.active) else { return };
        if !on {
            fx.release();
        } else if unit_on && (bypassed || !fx.has_tail()) {
            // Fresh start, e.g. the beatmasher re-arms on the next beat.
            fx.reset();
        }
    }

    pub fn slot_on(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(|s| s.enabled)
    }

    /// How much of `slot`'s effect is heard, `0..=1` (its knob in group mode).
    pub fn set_amount(&mut self, slot: usize, v: f32) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.amount.set_target(clamp01(v));
        }
    }

    /// Dry/wet knob, `0..=1` (linear crossfade).
    pub fn set_dry_wet(&mut self, v: f32) {
        self.dry_wet.set_target(clamp01(v));
    }

    /// Selects effect `effect_index` (see [`effect_names`](Self::effect_names))
    /// for `slot`. Invalid indices are ignored.
    pub fn select_effect(&mut self, slot: usize, effect_index: usize) {
        let idle = self.idle;
        let Some(s) = self.slots.get_mut(slot) else { return };
        if effect_index >= EFFECT_NAMES.len() || effect_index == s.selected {
            return;
        }
        s.selected = effect_index;
        if idle {
            s.mix.set_immediate(0.0);
        }
        s.complete_switch();
    }

    /// Selected effect index of `slot` (0 = none).
    pub fn slot_effect(&self, slot: usize) -> usize {
        self.slots.get(slot).map_or(0, |s| s.selected)
    }

    pub fn knob_names(&self, slot: usize) -> [&'static str; 3] {
        self.slots
            .get(slot)
            .and_then(|s| s.effects.get(s.selected.checked_sub(1)?))
            .map_or([""; 3], |fx| fx.knob_names())
    }

    pub fn button_names(&self, slot: usize) -> [&'static str; 3] {
        self.slots
            .get(slot)
            .and_then(|s| s.effects.get(s.selected.checked_sub(1)?))
            .map_or([""; 3], |fx| fx.button_names())
    }

    /// Sets knob `idx` of the effect selected in `slot`.
    pub fn set_knob(&mut self, slot: usize, idx: usize, v: f32) {
        if let Some(s) = self.slots.get_mut(slot) {
            if let Some(fx) = s.effect(s.selected) {
                fx.set_knob(idx, v);
            }
        }
    }

    /// Sets button `idx` of the effect selected in `slot`.
    pub fn set_button(&mut self, slot: usize, idx: usize, on: bool) {
        if let Some(s) = self.slots.get_mut(slot) {
            if let Some(fx) = s.effect(s.selected) {
                fx.set_button(idx, on);
            }
        }
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]], ctx: &FxCtx) {
        if self.idle {
            return;
        }
        for (i, chunk) in buf.chunks_mut(MAX_CHUNK).enumerate() {
            self.process_chunk(chunk, &ctx.advanced(i * MAX_CHUNK));
        }
    }

    fn process_chunk(&mut self, buf: &mut [[f32; 2]], ctx: &FxCtx) {
        let n = buf.len();
        let (wet, dry, gain) = (&mut self.wet[..n], &mut self.slot_dry[..n], &mut self.in_gain[..n]);
        for ((w, g), x) in wet.iter_mut().zip(gain.iter_mut()).zip(buf.iter()) {
            *g = self.input.tick();
            *w = [x[0] * *g, x[1] * *g];
        }
        for slot in &mut self.slots {
            slot.process(wet, dry, ctx, self.on, self.idle_after);
        }
        let mut peak = 0f32;
        for ((x, w), g) in buf.iter_mut().zip(wet.iter()).zip(gain.iter()) {
            let dw = self.dry_wet.tick();
            let dry_gain = 1.0 - g * dw;
            *x = [x[0] * dry_gain + w[0] * dw, x[1] * dry_gain + w[1] * dw];
            peak = peak.max(w[0].abs()).max(w[1].abs());
        }
        if !self.on && self.input.is_settled() && peak < SILENCE {
            self.silent_frames += n;
            if self.silent_frames >= self.idle_after {
                self.idle = true;
            }
        } else {
            self.silent_frames = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Rng, all_finite, ctx, sine};

    const SR: f32 = 48_000.0;

    fn run(unit: &mut FxUnit, buf: &mut [[f32; 2]], beat: &mut f64) {
        for b in buf.chunks_mut(128) {
            let c = ctx(120.0, *beat);
            assert_no_alloc::assert_no_alloc(|| unit.process(b, &c));
            *beat = c.beat_at(b.len());
        }
    }

    #[test]
    fn off_and_idle_is_transparent() {
        let mut u = FxUnit::new(SR);
        let input = sine(SR, 440.0, 0.5, 4800);
        let mut buf = input.clone();
        let mut beat = 0.0;
        run(&mut u, &mut buf, &mut beat);
        assert_eq!(buf, input);
        // On with no effects: dry/wet 1 mutes nothing but passes the (empty) chain.
        u.set_on(true);
        u.set_dry_wet(1.0);
        let mut buf = input.clone();
        run(&mut u, &mut buf, &mut beat);
        for (a, b) in buf[2000..].iter().zip(&input[2000..]) {
            assert!((a[0] - b[0]).abs() < 1e-6);
        }
    }

    fn rms(buf: &[[f32; 2]]) -> f32 {
        (buf.iter().map(|f| f[0] * f[0]).sum::<f32>() / buf.len() as f32).sqrt()
    }

    /// Delay, Reverb and Filter selected (the default FX 1), unit on, D/W full.
    fn group_unit() -> FxUnit {
        let mut u = FxUnit::new(SR);
        for (slot, e) in [1, 2, 3].into_iter().enumerate() {
            u.select_effect(slot, e);
        }
        u.set_dry_wet(1.0);
        u.set_on(true);
        u
    }

    #[test]
    fn switched_off_slots_pass_the_track() {
        // Before group mode, full D/W replaced the track with the 100 % wet
        // chain and made it quieter.
        let mut u = group_unit();
        let input = sine(SR, 440.0, 0.5, 9600);
        let (mut buf, mut beat) = (input.clone(), 0.0);
        run(&mut u, &mut buf, &mut beat);
        for (a, b) in buf[4800..].iter().zip(&input[4800..]) {
            assert!((a[0] - b[0]).abs() < 1e-4, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn amount_zero_is_transparent_and_amount_adds_the_effect() {
        let mut u = group_unit();
        u.set_slot_on(2, true); // Filter
        u.set_amount(2, 0.0);
        let input = sine(SR, 5000.0, 0.5, 9600);
        let (mut buf, mut beat) = (input.clone(), 0.0);
        run(&mut u, &mut buf, &mut beat);
        assert!((rms(&buf[4800..]) - rms(&input[4800..])).abs() < 1e-3);
        // Full amount: the low-pass filter takes the 5 kHz tone down.
        u.set_amount(2, 1.0);
        let mut buf = sine(SR, 5000.0, 0.5, 19_200);
        run(&mut u, &mut buf, &mut beat);
        assert!(rms(&buf[9600..]) < 0.5 * rms(&input[4800..]), "{}", rms(&buf[9600..]));
        u.set_slot_on(2, false);
        let mut buf = sine(SR, 5000.0, 0.5, 9600);
        run(&mut u, &mut buf, &mut beat);
        assert!((rms(&buf[4800..]) - rms(&input[4800..])).abs() < 1e-3);
    }

    #[test]
    fn delay_adds_echoes_on_top_of_the_track_and_rings_out() {
        let mut u = group_unit();
        u.set_knob(0, 0, 0.7); // 1 beat = 24000 frames at 120 BPM
        u.set_amount(0, 1.0);
        u.set_slot_on(0, true);
        let input = sine(SR, 440.0, 0.5, 12_000);
        let (mut buf, mut beat) = (input.clone(), 0.0);
        run(&mut u, &mut buf, &mut beat);
        // The track keeps its level while the effect is on.
        assert!((rms(&buf[2400..]) - rms(&input[2400..])).abs() < 0.02, "{}", rms(&buf[2400..]));
        // Switched off: the echo still arrives.
        u.set_slot_on(0, false);
        let mut silence = vec![[0.0f32; 2]; 36_000];
        run(&mut u, &mut silence, &mut beat);
        let tail = silence[12_000..24_000].iter().fold(0f32, |m, f| m.max(f[0].abs()));
        assert!(tail > 0.1, "{tail}");
        assert!(!u.slot_on(0));
    }

    #[test]
    fn delay_tail_rings_out_after_off_then_idles() {
        let mut u = FxUnit::new(SR);
        u.select_effect(0, 1);
        u.set_knob(0, 0, 0.7); // 1 beat
        u.set_knob(0, 1, 0.5);
        u.set_dry_wet(0.5);
        u.set_amount(0, 1.0);
        u.set_slot_on(0, true);
        u.set_on(true);
        let mut beat = 0.0;
        let mut buf = sine(SR, 440.0, 0.5, 12_000);
        run(&mut u, &mut buf, &mut beat);
        u.set_on(false);
        let mut silence = vec![[0.0f32; 2]; 48_000];
        run(&mut u, &mut silence, &mut beat);
        // Echo of the tone arrives 1 beat (24000 frames) after it started.
        let tail = silence[12_000..24_000].iter().fold(0f32, |m, f| m.max(f[0].abs()));
        assert!(tail > 0.1, "{tail}");
        assert!(!u.is_idle());
        let mut silence = vec![[0.0f32; 2]; 48_000 * 10];
        run(&mut u, &mut silence, &mut beat);
        assert!(u.is_idle());
    }

    #[test]
    fn switching_effects_is_smooth_and_never_allocates() {
        let mut u = FxUnit::new(SR);
        u.set_on(true);
        u.set_dry_wet(1.0);
        (0..SLOTS).for_each(|s| u.set_slot_on(s, true));
        let mut rng = Rng::new(11);
        let mut beat = 0.0;
        let mut prev = 0.0f32;
        let mut phase = 0.0f32;
        let mut buf = vec![[0.0f32; 2]; 128];
        for block in 0..3000 {
            if block % 37 == 0 {
                let slot = rng.below(SLOTS);
                assert_no_alloc::assert_no_alloc(|| u.select_effect(slot, rng.below(EFFECT_NAMES.len())));
            }
            if block % 11 == 0 {
                u.set_knob(rng.below(SLOTS), rng.below(3), rng.f32());
            }
            if block % 500 == 250 {
                u.set_on(!u.is_on());
            }
            for f in buf.iter_mut() {
                phase += 2.0 * std::f32::consts::PI * 50.0 / SR;
                *f = [phase.sin() * 0.3; 2];
            }
            run(&mut u, &mut buf, &mut beat);
            assert!(all_finite(&buf));
            if (0..SLOTS).all(|s| ![5, 6].contains(&u.slot_effect(s))) {
                // Without gater/beatmasher (which chop on purpose) a slow sine stays smooth.
                for f in &buf {
                    assert!((f[0] - prev).abs() < 0.2, "block {block}: {prev} -> {}", f[0]);
                    prev = f[0];
                }
            }
            prev = buf[buf.len() - 1][0];
        }
        assert_eq!(FxUnit::effect_names()[6], "Beatmasher");
        assert_eq!(u.knob_names(4), [""; 3]);
    }

    /// Two units with three effects each on 128-frame blocks; prints the
    /// CPU share of real time, also per effect.
    #[test]
    #[cfg_attr(debug_assertions, ignore = "timing is only meaningful in release")]
    fn performance() {
        crate::flush_denormals();
        let mut units = [FxUnit::new(SR), FxUnit::new(SR)];
        for (u, fx) in units.iter_mut().zip([[1, 2, 4], [3, 5, 6]]) {
            for (slot, e) in fx.into_iter().enumerate() {
                u.select_effect(slot, e);
                u.set_slot_on(slot, true);
            }
            u.set_on(true);
            u.set_dry_wet(0.7);
        }
        let mut rng = Rng::new(1);
        let input: Vec<[f32; 2]> = (0..48_000).map(|_| [rng.bipolar() * 0.5, rng.bipolar() * 0.5]).collect();
        let seconds = 20.0;
        let blocks = (seconds * SR / 128.0) as usize;
        let mut buf = [[0.0f32; 2]; 128];
        let mut time = |process: &mut dyn FnMut(&mut [[f32; 2]], &FxCtx)| {
            let start = std::time::Instant::now();
            for b in 0..blocks {
                let at = b * 128 % (input.len() - 128);
                buf.copy_from_slice(&input[at..at + 128]);
                process(&mut buf, &ctx(128.0, (b * 128) as f64 * 128.0 / 60.0 / SR as f64));
                assert!(buf[0][0].is_finite());
            }
            start.elapsed().as_secs_f64() / seconds as f64
        };
        let ratio = time(&mut |b, c| units.iter_mut().for_each(|u| u.process(b, c)));
        println!("2 FX units x 3 effects: {:.3} % of real time", ratio * 100.0);
        for (i, name) in EFFECT_NAMES.iter().enumerate().skip(1) {
            let mut fx = new_effect(i, SR).unwrap();
            println!("  {name}: {:.3} %", time(&mut |b, c| fx.process(b, c)) * 100.0);
        }
        assert!(ratio < 0.05, "{ratio}");
    }
}
