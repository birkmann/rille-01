//! LED feedback: turns control state into MIDI messages, sending only changes.

use crate::engine::ValueSource;
use crate::mapping::{Blink, InputTarget, Mapping, MidiSpec, OutputBinding};
use rille_core::ControlKind;

/// Remembers what was last sent to each output of one mapping.
#[derive(Clone, Debug, Default)]
pub struct FeedbackState {
    last: Vec<Option<u8>>,
}

impl FeedbackState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything is sent again on the next [`collect`](Self::collect), e.g.
    /// after (re)connecting a device.
    pub fn reset(&mut self) {
        self.last.clear();
    }

    /// Appends a message for every output whose value changed. Modifier
    /// sources count as off; see [`collect_with_modifiers`](Self::collect_with_modifiers).
    pub fn collect(&mut self, mapping: &Mapping, values: &dyn ValueSource, out: &mut Vec<[u8; 3]>) {
        self.collect_with_modifiers(mapping, values, &|_| false, out);
    }

    /// Like [`collect`](Self::collect), with `modifier` reporting whether a
    /// modifier is on (e.g. [`crate::MappingEngine::modifier`]).
    pub fn collect_with_modifiers(
        &mut self,
        mapping: &Mapping,
        values: &dyn ValueSource,
        modifier: &dyn Fn(&str) -> bool,
        out: &mut Vec<[u8; 3]>,
    ) {
        if self.last.len() != mapping.outputs.len() {
            self.last = vec![None; mapping.outputs.len()];
        }
        let beat_on = values.beat_phase().rem_euclid(1.0) < 0.5;
        let holds = |o: &OutputBinding| o.condition.as_ref().is_some_and(|c| modifier(&c.modifier) == c.value);
        // Messages taken over by a conditional output whose modifier holds.
        let taken: Vec<MidiSpec> = mapping.outputs.iter().filter(|o| holds(o)).map(|o| o.midi).collect();
        for (o, last) in mapping.outputs.iter().zip(&mut self.last) {
            let active = if o.condition.is_some() { holds(o) } else { !taken.contains(&o.midi) };
            if !active {
                // Sent again in full when it comes back.
                *last = None;
                continue;
            }
            let source = match &o.source {
                InputTarget::Control(t) => values.value(*t),
                InputTarget::Modifier(m) => f32::from(u8::from(modifier(m))),
                InputTarget::NextDeckLayout => 0.0,
            };
            let v = output_value(o, source, beat_on);
            if *last != Some(v) {
                *last = Some(v);
                if let Some(msg) = encode(o.midi, v) {
                    out.push(msg);
                }
            }
        }
    }

    /// Messages that switch every output off, e.g. before disconnecting.
    pub fn all_off(mapping: &Mapping, out: &mut Vec<[u8; 3]>) {
        out.extend(mapping.outputs.iter().filter(|o| o.condition.is_none()).filter_map(|o| encode(o.midi, o.off)));
    }
}

fn output_value(o: &OutputBinding, v: f32, beat_on: bool) -> u8 {
    if o.raw {
        return v.round().clamp(0.0, 127.0) as u8;
    }
    let v = match o.db_floor {
        // Linear amplitude to its place between the floor and 0 dB.
        Some(floor) if v > 0.0 => ((20.0 * v.log10() - floor) / -floor).clamp(0.0, 1.0),
        Some(_) => 0.0,
        None => v,
    };
    if let Some(t) = o.threshold {
        return if v >= t { o.on } else { o.off };
    }
    if o.source.control().is_some_and(|t| t.control.kind() == ControlKind::Continuous) {
        let (on, off) = (f32::from(o.on), f32::from(o.off));
        return (off + v.clamp(0.0, 1.0) * (on - off)).round() as u8;
    }
    let lit = v > 0.5 && (o.blink != Some(Blink::Beat) || beat_on);
    if lit { o.on } else { o.off }
}

fn encode(midi: MidiSpec, v: u8) -> Option<[u8; 3]> {
    let ch = midi.channel0() & 0x0f;
    match midi {
        MidiSpec::Note { number, .. } => Some([0x90 | ch, number, v]),
        MidiSpec::Cc { number, .. } => Some([0xb0 | ch, number, v]),
        MidiSpec::Cc14 { .. } | MidiSpec::PitchBend { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn conditional_outputs_take_over_while_their_modifier_holds() {
        let m = Mapping::from_toml(
            r#"
            name = "display"
            device = "x"
            [[output]]
            source = "deck.C.remix_page"
            midi = { type = "cc", channel = 3, number = 0 }
            raw = true
            [[output]]
            source = "deck.C.remix_capture_source"
            midi = { type = "cc", channel = 3, number = 0 }
            raw = true
            condition = { modifier = "capture" }
            "#,
        )
        .unwrap();
        let (mut fb, mut v, mut out) = (FeedbackState::new(), ValueMap::default(), Vec::new());
        v.set(ControlTarget::deck(2, Control::RemixPage), 2.0);
        v.set(ControlTarget::deck(2, Control::RemixCaptureSource), 100.0);
        fb.collect_with_modifiers(&m, &v, &|_| false, &mut out);
        assert_eq!(out, [[0xb2, 0, 2]]);
        out.clear();
        fb.collect_with_modifiers(&m, &v, &|n| n == "capture", &mut out);
        assert_eq!(out, [[0xb2, 0, 100]]);
        out.clear();
        fb.collect_with_modifiers(&m, &v, &|_| false, &mut out);
        assert_eq!(out, [[0xb2, 0, 2]], "the page comes back on release");
        out.clear();
        FeedbackState::all_off(&m, &mut out);
        assert_eq!(out.len(), 1);
    }

    use super::*;
    use crate::engine::ValueMap;
    use rille_core::{Control, ControlTarget};

    fn mapping() -> Mapping {
        Mapping::from_toml(
            r#"
            name = "fb"
            device = "x"
            [[output]]
            source = "deck.A.play"
            midi = { type = "note", channel = 1, number = 11 }
            [[output]]
            source = "deck.A.sync"
            midi = { type = "note", channel = 1, number = 12 }
            on = 100
            off = 5
            blink = "beat"
            [[output]]
            source = "deck.A.volume"
            midi = { type = "cc", channel = 2, number = 7 }
            "#,
        )
        .unwrap()
    }

    #[test]
    fn sends_only_changes() {
        let m = mapping();
        let mut fb = FeedbackState::new();
        let mut v = ValueMap::default();
        let play = ControlTarget::deck(0, Control::Play);
        v.set(ControlTarget::deck(0, Control::Volume), 0.0);
        let mut out = Vec::new();
        fb.collect(&m, &v, &mut out);
        assert_eq!(out, [[0x90, 11, 0], [0x90, 12, 5], [0xb1, 7, 0]]);
        out.clear();
        fb.collect(&m, &v, &mut out);
        assert!(out.is_empty());
        v.set(play, 1.0);
        v.set(ControlTarget::deck(0, Control::Volume), 0.5);
        fb.collect(&m, &v, &mut out);
        assert_eq!(out, [[0x90, 11, 127], [0xb1, 7, 64]]);
        out.clear();
        fb.reset();
        fb.collect(&m, &v, &mut out);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn thresholds_make_a_level_meter() {
        let m = Mapping::from_toml(
            r#"
            name = "vu"
            device = "x"
            [[output]]
            source = "deck.B.meter"
            midi = { type = "note", channel = 1, number = 1 }
            threshold = 0.25
            [[output]]
            source = "deck.B.meter"
            midi = { type = "note", channel = 1, number = 2 }
            threshold = 0.5
            off = 3
            "#,
        )
        .unwrap();
        let (mut fb, mut v, mut out) = (FeedbackState::new(), ValueMap::default(), Vec::new());
        v.set(ControlTarget::deck(1, Control::Meter), 0.3);
        fb.collect(&m, &v, &mut out);
        assert_eq!(out, [[0x90, 1, 127], [0x90, 2, 3]]);
        out.clear();
        v.set(ControlTarget::deck(1, Control::Meter), 0.5);
        fb.collect(&m, &v, &mut out);
        assert_eq!(out, [[0x90, 2, 127]]);
    }

    #[test]
    fn blinks_with_the_beat() {
        let m = mapping();
        let mut fb = FeedbackState::new();
        let mut v = ValueMap::default();
        v.set(ControlTarget::deck(0, Control::Sync), 1.0);
        let mut out = Vec::new();
        let mut sync_led = |phase: f64| {
            v.beat_phase = phase;
            out.clear();
            fb.collect(&m, &v, &mut out);
            out.iter().find(|m| m[1] == 12).map(|m| m[2])
        };
        assert_eq!(sync_led(0.1), Some(100));
        assert_eq!(sync_led(0.3), None); // unchanged
        assert_eq!(sync_led(0.7), Some(5));
        assert_eq!(sync_led(1.2), Some(100));
    }

    #[test]
    fn modifier_leds_and_db_meters() {
        let m = Mapping::from_toml(
            r#"
            name = "amx-ish"
            device = "x"
            [[output]]
            source = "modifier:touch"
            midi = { type = "note", channel = 1, number = 25 }
            [[output]]
            source = "global.main_meter.1"
            midi = { type = "cc", channel = 1, number = 62 }
            on = 80
            db_floor = -60
            "#,
        )
        .unwrap();
        let (mut fb, mut v, mut out) = (FeedbackState::new(), ValueMap::default(), Vec::new());
        let meter = ControlTarget::global(Control::MainMeter(1));
        v.set(meter, 0.001); // -60 dBFS
        fb.collect(&m, &v, &mut out);
        assert_eq!(out, [[0x90, 25, 0], [0xb0, 62, 0]]);
        out.clear();
        v.set(meter, 0.1); // -20 dBFS: two thirds up
        fb.collect_with_modifiers(&m, &v, &|n| n == "touch", &mut out);
        assert_eq!(out, [[0x90, 25, 127], [0xb0, 62, 53]]);
        out.clear();
        v.set(meter, 2.0);
        fb.collect_with_modifiers(&m, &v, &|n| n == "touch", &mut out);
        assert_eq!(out, [[0xb0, 62, 80]]);
    }

    #[test]
    fn raw_values_pass_through() {
        let m = Mapping::from_toml(
            r#"
            name = "f1-ish"
            device = "x"
            [[output]]
            source = "deck.C.remix_pad.1"
            midi = { type = "note", channel = 2, number = 1 }
            raw = true
            [[output]]
            source = "deck.C.remix_page"
            midi = { type = "cc", channel = 3, number = 0 }
            raw = true
            off = 127
            "#,
        )
        .unwrap();
        assert_eq!(Mapping::from_toml(&m.to_toml()).unwrap(), m);
        let (mut fb, mut v, mut out) = (FeedbackState::new(), ValueMap::default(), Vec::new());
        let (pad, page) = (ControlTarget::deck(2, Control::RemixPad(1)), ControlTarget::deck(2, Control::RemixPage));
        v.set(pad, 35.0);
        v.set(page, 2.0);
        fb.collect(&m, &v, &mut out);
        assert_eq!(out, [[0x91, 1, 35], [0xb2, 0, 2]]);
        for (x, want) in [(34.4, 34), (-3.0, 0), (300.0, 127), (f32::NAN, 0)] {
            out.clear();
            v.set(pad, x);
            fb.collect(&m, &v, &mut out);
            assert_eq!(out, [[0x91, 1, want]], "{x}");
        }
        out.clear();
        FeedbackState::all_off(&m, &mut out);
        assert_eq!(out, [[0x91, 1, 0], [0xb2, 0, 127]], "off blanks the display");

        for extra in ["threshold = 0.5", "db_floor = -60", "blink = \"beat\""] {
            let src = format!(
                "name = \"x\"\ndevice = \"x\"\n[[output]]\nsource = \"deck.A.remix_page\"\n\
                 midi = {{ type = \"cc\", channel = 3, number = 0 }}\nraw = true\n{extra}"
            );
            let e = Mapping::from_toml(&src).unwrap_err();
            assert!(e.to_string().contains("raw cannot"), "{extra}: {e}");
        }
    }
}
