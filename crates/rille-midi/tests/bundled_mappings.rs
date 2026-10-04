//! Every file in the repository's `mappings/` directory must load, and no two
//! inputs may listen to the same message under the same condition.

use rille_core::{ControlEvent, ControlValue};
use rille_midi::store::{MappingStore, load_file};
use rille_midi::{Mapping, MappingEngine, MidiSpec, ValueMap};
use std::collections::HashSet;
use std::path::PathBuf;

fn bundled_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mappings")
}

#[test]
fn all_bundled_mappings_load() {
    let dir = bundled_dir();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.retain(|p| p.extension().is_some_and(|e| e == "toml"));
    assert!(files.len() >= 3, "{files:?}");
    for path in &files {
        let m = load_file(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!m.inputs.is_empty(), "{}", path.display());
        let mut seen = HashSet::new();
        // A tap button shares its message with the modifier it holds.
        for b in m.inputs.iter().filter(|b| b.tap) {
            let holds = |h: &&rille_midi::InputBinding| {
                matches!(h.target, rille_midi::InputTarget::Modifier(_))
                    && h.midi == b.midi
                    && h.condition == b.condition
            };
            assert!(m.inputs.iter().any(|h| holds(&h)), "{}: tap {} holds no modifier", path.display(), b.target);
        }
        for b in m.inputs.iter().filter(|b| !b.tap) {
            let keys = match b.midi {
                MidiSpec::Cc14 { channel, number } => {
                    vec![MidiSpec::Cc { channel, number }, MidiSpec::Cc { channel, number: number + 32 }]
                }
                other => vec![other],
            };
            for k in keys {
                assert!(seen.insert((k, b.condition.clone())), "{}: {k:?} used twice", path.display());
            }
        }
        // MIDI learn saves a copy; it must read back the same.
        assert_eq!(Mapping::from_toml(&m.to_toml()).as_ref(), Ok(&m), "{}", path.display());
        // Smoke: the engine accepts the mapping.
        let mut engine = MappingEngine::new(m);
        let mut out = Vec::new();
        engine.handle(&[0x90, 11, 127], std::time::Instant::now(), &ValueMap::default(), &mut out);
    }
    let store = MappingStore::load(&[&dir]);
    assert!(store.errors().is_empty(), "{:?}", store.errors());
    assert_eq!(store.find("DDJ-400:DDJ-400 MIDI 1 24:0").unwrap().name, "Pioneer DDJ-400");
    let herc = "DJControl Inpulse 200:DJControl Inpulse 200 MIDI 1 28:0";
    assert_eq!(store.find(herc).unwrap().name, "Hercules DJControl Inpulse 200");
    assert!(store.find("Midi Through:Midi Through Port-0 14:0").is_none());
    let k2 = "XONE:K2:XONE:K2 MIDI 1 20:0";
    assert_eq!(store.find(k2).unwrap().name, "Allen & Heath Xone:K2 (CABD)");
    assert!(store.by_name("Allen & Heath Xone:K2 (ABCD)").is_some());
    let z1 = store.find("Traktor Kontrol Z1 HID (A1B2C3D4)").unwrap();
    assert_eq!(z1.name, "Traktor Kontrol Z1 (AB)");
    assert_eq!(store.find("Traktor Kontrol X1 MK2 HID").unwrap().name, "Traktor Kontrol X1 MK2 (AB)");
    assert!(store.find("Traktor Kontrol X1 MK2 MIDI 1 24:0").is_none(), "MIDI mode is a different device");
    assert_eq!(store.find("Traktor Kontrol X1 MK1 HID (ABC123)").unwrap().name, "Traktor Kontrol X1 MK1 (AB)");
    assert_eq!(store.find("Traktor Kontrol F1 HID (1A2B3C4D)").unwrap().name, "Traktor Kontrol F1 (C)");
    // One HID layout per device, from the mapping files; each mapping's
    // device pattern matches the port name its layout gives.
    let hid: Vec<String> = store.hid_layouts().iter().map(|l| l.name.clone()).collect();
    assert_eq!(
        hid,
        [
            "Maschine Mikro MK2",
            "Traktor Kontrol F1",
            "Traktor Kontrol X1 MK1",
            "Traktor Kontrol X1 MK2",
            "Traktor Kontrol Z1"
        ]
    );
    for l in store.hid_layouts() {
        let m = store.find(&format!("{} HID (SERIAL)", l.name)).unwrap();
        assert_eq!(m.hid.as_ref().map(|h| &h.name), Some(&l.name));
    }
}

/// HID reports through the translator and the mapping: the targets pressed
/// or moved, in order.
fn hid_targets(m: &Mapping, reports: &[Vec<u8>]) -> Vec<String> {
    let layout = std::sync::Arc::new(m.hid.clone().expect("a HID mapping"));
    let mut hid = rille_midi::hid::HidTranslator::new(layout);
    let mut engine = MappingEngine::new(m.clone());
    let (mut msgs, mut out, values) = (Vec::new(), Vec::new(), ValueMap::default());
    let t = std::time::Instant::now();
    for r in reports {
        hid.input(r, &mut msgs);
    }
    for msg in &msgs {
        engine.handle(msg, t, &values, &mut out);
    }
    let pressed_or_moved = |e: &&ControlEvent| !matches!(e.value, ControlValue::Press(false));
    out.iter().filter(pressed_or_moved).map(|e| e.target.to_string()).collect()
}

#[test]
fn maschine_mikro_mk2() {
    use rille_core::remix::led_code;
    use rille_core::{Control, ControlTarget};
    let store = MappingStore::load(&[bundled_dir()]);
    let m = store.find("Maschine Mikro MK2 HID (B1E971E8)").unwrap();
    // Button report: bit b of bytes 1-4 is button b; byte 5 the encoder.
    let buttons = |bits: &[u8], enc: u8| {
        let mut v = vec![0x01, 0, 0, 0, 0, enc];
        bits.iter().for_each(|&b| v[1 + usize::from(b / 8)] |= 1 << (b % 8));
        v
    };
    // Pad report: 32 values, the pad in the top 4 bits (0 = top left, row
    // by row, like the LEDs).
    let pads = |pad: u16, pressure: u16| {
        let mut v = vec![0x20];
        for k in 0..32u16 {
            let p = k % 16;
            v.extend_from_slice(&(p << 12 | if p == pad { pressure } else { 0 }).to_le_bytes());
        }
        v
    };
    let hit = |pad| [pads(pad, 3800), pads(pad, 3800), pads(pad, 3800), pads(pad, 0)];
    let (shift, erase, rec, sampling, pad_mode, f1, pattern) = (0, 1, 2, 13, 29, 23, 30);
    let mut r = vec![buttons(&[], 0)];
    // Pad 13 (top left): step 1.
    r.extend(hit(0));
    // PAD MODE: pad 13 plays BD, pad 1 (bottom left) mutes CP.
    r.extend([buttons(&[pad_mode], 0), buttons(&[], 0)]);
    r.extend(hit(0));
    r.extend(hit(12));
    // SHIFT + pad 1: UNDO, as printed.
    r.push(buttons(&[shift], 0));
    r.extend(hit(12));
    // The encoder: the filter; after F1 the selected instrument's level.
    r.extend([buttons(&[], 1), buttons(&[f1], 1), buttons(&[], 1), buttons(&[], 2)]);
    // SHIFT+REC: count-in; ERASE+REC: replace.
    r.extend([buttons(&[shift], 2), buttons(&[shift, rec], 2), buttons(&[], 2)]);
    r.extend([buttons(&[erase], 2), buttons(&[erase, rec], 2), buttons(&[], 2)]);
    // SAMPLING + pad 13: load into BD; a tap on SAMPLING: headphones.
    r.push(buttons(&[sampling], 2));
    r.extend(hit(0));
    r.extend([buttons(&[], 2), buttons(&[sampling], 2), buttons(&[], 2)]);
    // PATTERN held + pad 16 (top right): pattern 4.
    r.push(buttons(&[pattern], 2));
    r.extend(hit(3));
    assert_eq!(
        hid_targets(m, &r),
        [
            "drum.step.1",
            "drum.trigger.1",
            "drum.inst_mute.5",
            "drum.undo",
            "drum.filter",
            "drum.sel_level",
            "drum.count_in",
            "drum.replace",
            "drum.load_selected.1",
            "drum.pfl",
            "drum.pattern.4"
        ]
    );

    // LEDs: step 1 bright red, GROUP in the selected instrument's colour.
    let mut values = ValueMap::default();
    let code = led_code(1, true);
    values.set(ControlTarget::drum(Control::DrumStepLed(1)), f32::from(code));
    values.set(ControlTarget::drum(Control::DrumSelLed), f32::from(code));
    let (mut fb, mut msgs) = (rille_midi::FeedbackState::new(), Vec::new());
    fb.collect(m, &values, &mut msgs);
    let mut hid = rille_midi::hid::HidTranslator::new(std::sync::Arc::new(m.hid.clone().unwrap()));
    msgs.iter().for_each(|msg| hid.output(*msg));
    let mut out = Vec::new();
    hid.flush(|report| out.push(report.to_vec()));
    let leds = &out[0];
    assert_eq!(leds.len(), 79);
    assert!(leds[31] > 3 * leds[32] && leds[31] > 3 * leds[33], "pad 13 red: {:?}", &leds[31..34]);
    assert_eq!(&leds[9..12], &leds[31..34], "GROUP lit like the pad");
    assert!(leds[34..79].iter().all(|&b| b == 0), "other pads off");
    assert_eq!(leds[19], 10, "PLAY dim while stopped");
    assert_eq!(out.len(), 1 + 4, "LEDs and the display's four chunks");
}

#[test]
fn traktor_hid_controllers() {
    let store = MappingStore::load(&[bundled_dir()]);
    let x1 = |r: &[(usize, u8)]| {
        let mut v = vec![0u8; 0x1F];
        v[0] = 0x01;
        r.iter().for_each(|&(i, b)| v[i] = b);
        v
    };
    // Left PLAY (0x16 bit 0), then SHIFT + right pad 1 (0x14 bit 2, 0x15 bit 7).
    let reports = [x1(&[]), x1(&[(0x16, 0x01)]), x1(&[]), x1(&[(0x14, 0x04)]), x1(&[(0x14, 0x04), (0x15, 0x80)])];
    let ab = store.by_name("Traktor Kontrol X1 MK2 (AB)").unwrap();
    let cd = store.by_name("Traktor Kontrol X1 MK2 (CD)").unwrap();
    let moved = |t: &Vec<String>| t.iter().filter(|t| !t.starts_with("fx.")).cloned().collect::<Vec<_>>();
    assert_eq!(moved(&hid_targets(ab, &reports)), ["deck.A.play", "deck.B.hotcue_delete.1"]);
    assert_eq!(moved(&hid_targets(cd, &reports)), ["deck.C.play", "deck.D.hotcue_delete.1"]);
    // The left deck encoder turning clockwise doubles the loop.
    let enc = hid_targets(ab, &[x1(&[(0x11, 0x05)]), x1(&[(0x11, 0x06)])]);
    assert!(enc.ends_with(&["deck.A.loop_double".to_string()]), "{enc:?}");
    // SYNC held (note 26: 0x16 bit 2) + the left encoder one tick on changes
    // the tempo, without syncing or doubling the loop; a tap syncs.
    let sync = [x1(&[]), x1(&[(0x16, 0x04)]), x1(&[(0x16, 0x04), (0x11, 0x01)]), x1(&[(0x11, 0x01)])];
    assert_eq!(moved(&hid_targets(ab, &sync)), ["deck.A.tempo"]);
    assert_eq!(moved(&hid_targets(ab, &[x1(&[]), x1(&[(0x16, 0x04)]), x1(&[])])), ["deck.A.sync"]);
    // SHIFT + browse press (note 33: 0x17 bit 1) switches between AB and CD;
    // browse press alone does not.
    let switches = |reports: &[Vec<u8>]| {
        let mut hid = rille_midi::hid::HidTranslator::new(std::sync::Arc::new(ab.hid.clone().unwrap()));
        let mut engine = MappingEngine::new(ab.clone());
        let (mut msgs, mut out) = (Vec::new(), Vec::new());
        reports.iter().for_each(|r| hid.input(r, &mut msgs));
        let t = std::time::Instant::now();
        msgs.iter().for_each(|m| engine.handle(m, t, &ValueMap::default(), &mut out));
        engine.take_next_deck_layout()
    };
    assert!(!switches(&[x1(&[]), x1(&[(0x17, 0x02)])]));
    assert!(switches(&[x1(&[]), x1(&[(0x14, 0x04)]), x1(&[(0x14, 0x04), (0x17, 0x02)])]));
    assert_eq!(store.next_deck_layout(&ab.name).map(|m| &m.name), Some(&cd.name));
    assert_eq!(store.next_deck_layout(&cd.name).map(|m| &m.name), Some(&ab.name));

    // Z1: MODE (bit 1) held turns FX 1 (bit 2) into play for deck A.
    let z1 = |knobs: &[(usize, u16)], buttons: u8| {
        let mut v = vec![0x01];
        let mut k = [0u16; 14];
        knobs.iter().for_each(|&(i, x)| k[i] = x);
        k.iter().for_each(|x| v.extend(x.to_le_bytes()));
        v.push(buttons);
        v
    };
    let z1_map = store.by_name("Traktor Kontrol Z1 (AB)").unwrap();
    let targets = hid_targets(z1_map, &[z1(&[], 0), z1(&[], 0x02), z1(&[(11, 4095)], 0x06)]);
    assert!(targets.contains(&"deck.A.play".to_string()), "{targets:?}");
    assert!(!targets.contains(&"deck.A.fx_assign.1".to_string()), "{targets:?}");

    // Level meters: channel 1 at -5 dBFS lights -30 .. 0 (bytes 1-5); channel 2
    // at full scale lights all of bytes 8-14.
    let mut values = ValueMap::default();
    values.set(rille_core::ControlTarget::deck(0, rille_core::Control::Meter), 0.56);
    values.set(rille_core::ControlTarget::deck(1, rille_core::Control::Meter), 1.0);
    let (mut fb, mut msgs) = (rille_midi::FeedbackState::new(), Vec::new());
    fb.collect(z1_map, &values, &mut msgs);
    let mut hid = rille_midi::hid::HidTranslator::new(std::sync::Arc::new(z1_map.hid.clone().unwrap()));
    msgs.iter().for_each(|m| hid.output(*m));
    let mut report = Vec::new();
    hid.flush(|r| report = r.to_vec());
    let lit: Vec<bool> = report[1..=14].iter().map(|&b| b == 127).collect();
    let want = [true, true, true, true, true, false, false, true, true, true, true, true, true, true];
    assert_eq!(lit, want, "{:?}", &report[..15]);
}

#[test]
fn traktor_kontrol_x1_mk1() {
    use rille_core::{Control, ControlTarget};
    use rille_midi::caiaq::Report;
    let store = MappingStore::load(&[bundled_dir()]);
    let ab = store.by_name("Traktor Kontrol X1 MK1 (AB)").unwrap();
    let cd = store.by_name("Traktor Kontrol X1 MK1 (CD)").unwrap();
    // Input events (type, code, value) to a report per frame, as the caiaq
    // reader does: key BTN_MISC + n is note n.
    let (key, abs) = (|n: u16, v: i32| (1, 0x100 + n, v), |code: u16, v: i32| (3, code, v));
    let frames = |frames: &[&[(u16, u16, i32)]]| {
        let mut r = Report::new(0x01);
        let mut out = vec![r.bytes().to_vec()];
        for f in frames {
            f.iter().for_each(|&(kind, code, v)| r.apply(kind, code, v));
            out.push(r.bytes().to_vec());
        }
        out
    };
    let not_fx = |t: Vec<String>| t.into_iter().filter(|t| !t.starts_with("fx.")).collect::<Vec<_>>();

    // Left PLAY; HOTCUE on, then left IN sets hotcue 1 and SHIFT + IN deletes
    // it; HOTCUE off, IN is loop in again.
    let reports = frames(&[
        &[key(0, 1)],
        &[key(0, 0)],
        &[key(39, 1)],
        &[key(39, 0)],
        &[key(20, 1)],
        &[key(20, 0)],
        &[key(36, 1)],
        &[key(20, 1)],
        &[key(20, 0), key(36, 0)],
        &[key(39, 1)],
        &[key(39, 0)],
        &[key(20, 1)],
    ]);
    let want = ["deck.A.play", "deck.A.hotcue.1", "deck.A.hotcue_delete.1", "deck.A.loop_in"];
    assert_eq!(not_fx(hid_targets(ab, &reports)), want);
    assert_eq!(not_fx(hid_targets(cd, &reports[..2])), ["deck.C.play"]);
    // The left loop encoder (ABS_Z) one tick on doubles the loop; the right
    // browse encoder (ABS_Y) scrolls; FX 2 knob 3 (ABS_HAT3Y) is CC 7.
    let moved = hid_targets(ab, &frames(&[&[abs(0x02, 1)], &[abs(0x01, 15)], &[abs(0x17, 4095)]]));
    for t in ["deck.A.loop_double", "global.scroll", "fx.2.knob.3"] {
        assert!(moved.contains(&t.to_string()), "{t} not in {moved:?}");
    }
    // SHIFT + HOTCUE switches between AB and CD; HOTCUE alone does not.
    let switches = |reports: &[Vec<u8>]| {
        let mut hid = rille_midi::hid::HidTranslator::new(std::sync::Arc::new(ab.hid.clone().unwrap()));
        let mut engine = MappingEngine::new(ab.clone());
        let (mut msgs, mut out) = (Vec::new(), Vec::new());
        reports.iter().for_each(|r| hid.input(r, &mut msgs));
        let t = std::time::Instant::now();
        msgs.iter().for_each(|m| engine.handle(m, t, &ValueMap::default(), &mut out));
        (engine.take_next_deck_layout(), engine.modifier("hotcue"))
    };
    assert_eq!(switches(&frames(&[&[key(39, 1)]])), (false, true));
    assert_eq!(switches(&frames(&[&[key(36, 1)], &[key(39, 1)]])), (true, false));

    // LEDs: deck A playing lights PLAY (byte 24); FX 1 ON (byte 8) dimmed.
    // In hotcue mode PLAY shows hotcue 4 instead.
    let mut values = ValueMap::default();
    values.set(ControlTarget::deck(0, Control::Play), 1.0);
    let leds = |hotcue: bool| {
        let (mut fb, mut msgs) = (rille_midi::FeedbackState::new(), Vec::new());
        fb.collect_with_modifiers(ab, &values, &|m| hotcue && m == "hotcue", &mut msgs);
        let mut hid = rille_midi::hid::HidTranslator::new(std::sync::Arc::new(ab.hid.clone().unwrap()));
        msgs.iter().for_each(|m| hid.output(*m));
        let mut report = Vec::new();
        hid.flush(|r| report = r.to_vec());
        report
    };
    let plain = leds(false);
    assert_eq!((plain[0], plain.len(), plain[24], plain[8], plain[29]), (0x0C, 32, 127, 5, 5));
    let hot = leds(true);
    assert_eq!((hot[24], hot[29]), (5, 127), "hotcue 4 not set; HOTCUE lit");
}

/// Presses (note on + off, channel 15) and returns the targets pressed.
fn presses(m: &Mapping, notes: &[u8]) -> Vec<String> {
    let mut engine = MappingEngine::new(m.clone());
    let (mut out, t, values) = (Vec::new(), std::time::Instant::now(), ValueMap::default());
    for &n in notes {
        engine.handle(&[0x9e, n, 127], t, &values, &mut out);
        engine.handle(&[0x8e, n, 0], t, &values, &mut out);
    }
    let pressed = |e: &&ControlEvent| e.value == ControlValue::Press(true);
    out.iter().filter(pressed).map(|e| e.target.to_string()).collect()
}

#[test]
fn xone_k2_deck_layouts() {
    let store = MappingStore::load(&[bundled_dir()]);
    let cabd = store.by_name("Allen & Heath Xone:K2 (CABD)").unwrap();
    let abcd = store.by_name("Allen & Heath Xone:K2 (ABCD)").unwrap();
    // PLAY is notes 40-43, left to right.
    let play = [40, 41, 42, 43];
    assert_eq!(presses(cabd, &play), ["deck.C.play", "deck.A.play", "deck.B.play", "deck.D.play"]);
    assert_eq!(presses(abcd, &play), ["deck.A.play", "deck.B.play", "deck.C.play", "deck.D.play"]);
    // Hotcue 1 row, the second column.
    assert_eq!(presses(cabd, &[37]), ["deck.A.hotcue.1"]);

    // SHIFT (note 15) held: reverse; BROWSE (note 14) held: load.
    let mut engine = MappingEngine::new(cabd.clone());
    let (mut out, t, values) = (Vec::new(), std::time::Instant::now(), ValueMap::default());
    for msg in [[0x9e, 15, 127], [0x9e, 41, 127], [0x8e, 15, 0], [0x9e, 14, 127], [0x9e, 40, 127]] {
        engine.handle(&msg, t, &values, &mut out);
    }
    let targets: Vec<_> = out.iter().map(|e| e.target.to_string()).collect();
    assert_eq!(targets, ["deck.A.reverse", "deck.C.load_selected"]);

    // The play LED of the second column (green = note 41 + 72) follows deck A.
    let led = cabd.outputs.iter().find(|o| o.midi == MidiSpec::Note { channel: 15, number: 113 }).unwrap();
    assert_eq!(led.source.to_string(), "deck.A.play");
}

/// Raw messages through a mapping: every event as `target=value`.
fn events(m: &Mapping, msgs: &[[u8; 3]]) -> Vec<String> {
    let mut engine = MappingEngine::new(m.clone());
    let (mut out, t, values) = (Vec::new(), std::time::Instant::now(), ValueMap::default());
    for msg in msgs {
        engine.handle(msg, t, &values, &mut out);
    }
    out.iter()
        .map(|e| match e.value {
            ControlValue::Press(p) => format!("{}={}", e.target, if p { "down" } else { "up" }),
            ControlValue::Absolute(v) => format!("{}={v:.3}", e.target),
            ControlValue::Delta(d) => format!("{}={d:+.3}", e.target),
            ControlValue::Hit(v) => format!("{}=hit {v:.2}", e.target),
        })
        .collect()
}

#[test]
fn xone_96_with_k2_on_x_link() {
    let store = MappingStore::load(&[bundled_dir()]);
    let port = "XONE:96:XONE:96 MIDI 1 24:0";
    let m = store.find(port).unwrap();
    assert_eq!(m.name, "Allen & Heath Xone:96 (CABD)");
    assert_eq!(store.find("XONE:K2:XONE:K2 MIDI 1 20:0").unwrap().name, "Allen & Heath Xone:K2 (CABD)");
    // Channel 2's fader (CC 1, channel 16) is deck A's; the crossfader on
    // CC 4 or 5; no soft takeover, rille follows the hardware.
    let moved = events(m, &[[0xbf, 1, 0], [0xbf, 0, 127], [0xbf, 4, 0], [0xbf, 5, 127]]);
    assert_eq!(
        moved,
        ["deck.A.volume=0.000", "deck.C.volume=1.000", "global.crossfader=0.000", "global.crossfader=1.000"]
    );
    let abcd = store.by_name("Allen & Heath Xone:96 (ABCD)").unwrap();
    assert_eq!(events(abcd, &[[0xbf, 0, 127]]), ["deck.A.volume=1.000"]);
    // The K2 on X:LINK (channel 15) follows the same deck layout, LEDs too.
    assert_eq!(presses(m, &[40, 41]), ["deck.C.play", "deck.A.play"]);
    assert_eq!(presses(abcd, &[40, 41]), ["deck.A.play", "deck.B.play"]);
    assert!(m.outputs.iter().any(|o| o.midi == MidiSpec::Note { channel: 15, number: 113 }));
    let mut seen = HashSet::new();
    for b in &m.inputs {
        assert!(seen.insert((b.midi, b.condition.clone())), "{:?} used twice", b.midi);
    }
}

#[test]
fn akai_amx() {
    let store = MappingStore::load(&[bundled_dir()]);
    assert_eq!(store.find("AMX:AMX MIDI 1 20:0").unwrap().name, "Akai AMX (AB)");
    let ab = store.by_name("Akai AMX (AB)").unwrap();
    let cd = store.by_name("Akai AMX (CD)").unwrap();
    let (on, off) = (|n: u8| [0x90, n, 127], |n: u8| [0x80, n, 0]);

    // PLAY 1/2, then SHIFT + CUE 1 (back to start), SHIFT + PLAY 2 (stutter).
    let msgs = [on(10), off(10), on(11), off(11), on(0), on(8), off(8), on(11), off(11), off(0)];
    let want = ["deck.A.play=down", "deck.A.play=up", "deck.B.play=down", "deck.B.play=up"];
    let shifted = ["deck.A.jump_start=down", "deck.A.jump_start=up", "deck.B.cup=down", "deck.B.cup=up"];
    assert_eq!(events(ab, &msgs), [&want[..], &shifted[..]].concat());
    assert_eq!(events(cd, &msgs[..2]), ["deck.C.play=down", "deck.C.play=up"]);

    // Touching TREBLE 1 does nothing until TOUCH switches touch mode on.
    let msgs = [on(17), off(17), on(25), off(25), on(17), off(17), on(24), off(24), on(25), off(25), on(17)];
    let touch = ["deck.A.eq_hi_kill=down", "deck.A.eq_hi_kill=up", "deck.B.filter_roll=down", "deck.B.filter_roll=up"];
    assert_eq!(events(ab, &msgs), touch);

    // SEARCH 2 held: BROWSE moves deck B (jog) instead of the list; SYNC 2
    // toggles its loop.
    let msgs = [[0xb0, 59, 1], on(3), [0xb0, 59, 127], on(7), off(7), off(3), [0xb0, 59, 1]];
    let want = ["global.scroll=+1.000", "deck.B.jog=-0.042", "deck.B.loop_toggle=down", "deck.B.loop_toggle=up"];
    assert_eq!(events(ab, &msgs), [&want[..], &["global.scroll=+1.000"]].concat());

    // 14-bit channel fader 2 (MSB CC 11, LSB CC 43) picked up at rille's 1.0: the
    // first MSB counts alone until an LSB has been seen, then pairs combine.
    // Then gain encoder 1 and the crossfader reverse switch (1 = on).
    let msgs = [[0xb0, 11, 127], [0xb0, 43, 127], [0xb0, 11, 64], [0xb0, 43, 0], [0xb0, 60, 1]];
    let want = ["deck.B.volume=1.000", "deck.B.volume=1.000", "deck.B.volume=0.500", "deck.A.gain=0.520"];
    assert_eq!(events(ab, &msgs), want);
    let rev = ["global.crossfader_reverse=down", "global.crossfader_reverse=up"];
    assert_eq!(events(ab, &[[0xb0, 58, 1], [0xb0, 58, 0]]), rev);

    // LEDs: PLAY 1, the TOUCH LED from the latched modifier, meters in dB.
    let mut values = ValueMap::default();
    values.set(rille_core::ControlTarget::deck(0, rille_core::Control::Play), 1.0);
    values.set(rille_core::ControlTarget::deck(1, rille_core::Control::Meter), 1.0);
    values.set(rille_core::ControlTarget::global(rille_core::Control::MainMeter(1)), 0.001);
    let mut engine = MappingEngine::new(ab.clone());
    engine.handle(&on(25), std::time::Instant::now(), &values, &mut Vec::new());
    let (mut fb, mut msgs) = (rille_midi::FeedbackState::new(), Vec::new());
    fb.collect_with_modifiers(ab, &values, &|m| engine.modifier(m), &mut msgs);
    for want in [[0x90, 10, 127], [0x90, 11, 0], [0x90, 25, 127], [0xb0, 65, 81], [0xb0, 62, 0], [0xb0, 64, 0]] {
        assert!(msgs.contains(&want), "{want:?} not in {msgs:?}");
    }
}

#[test]
fn traktor_kontrol_f1() {
    use rille_core::remix::led_code;
    use rille_core::{Control, ControlTarget};
    let store = MappingStore::load(&[bundled_dir()]);
    let c = store.by_name("Traktor Kontrol F1 (C)").unwrap();
    let a = store.by_name("Traktor Kontrol F1 (A)").unwrap();
    let f1 = |r: &[(usize, u8)]| {
        let mut v = vec![0u8; 22];
        v[0] = 0x01;
        r.iter().for_each(|&(i, b)| v[i] = b);
        v
    };
    // Pad 1 (byte 1 bit 7), then SHIFT (byte 3 bit 7) + pad 16 (byte 2 bit 0),
    // then SHIFT + the encoder (byte 5) one tick on.
    let reports = [
        f1(&[]),
        f1(&[(1, 0x80)]),
        f1(&[]),
        f1(&[(3, 0x80)]),
        f1(&[(3, 0x80), (2, 0x01)]),
        f1(&[(3, 0x80), (5, 0x01)]),
    ];
    // The first report sets every slot filter and volume (no soft takeover:
    // the F1's knobs and faders are where the slots are).
    let all = hid_targets(c, &reports);
    let (slots, rest): (Vec<_>, Vec<_>) =
        all.iter().cloned().partition(|t| t.contains("remix_filter") || t.contains("remix_volume"));
    for s in 1..=4 {
        assert!(
            slots.contains(&format!("deck.C.remix_volume.{s}")) && slots.contains(&format!("deck.C.remix_filter.{s}"))
        );
    }
    assert_eq!(rest, ["deck.C.remix_pad.1", "deck.C.remix_pad_delete.16", "deck.C.tempo"]);
    let first_pad = hid_targets(a, &reports[..2]);
    assert_eq!(first_pad.last().map(String::as_str), Some("deck.A.remix_pad.1"));
    // The encoder alone pages back across the wrap; STOP 1 (byte 4 bit 7) stops slot 1.
    let turn: Vec<String> = hid_targets(c, &[f1(&[(5, 2)]), f1(&[(5, 255), (4, 0x80)])])
        .into_iter()
        .filter(|t| !t.contains("remix_filter") && !t.contains("remix_volume"))
        .collect();
    assert_eq!(turn, ["deck.C.remix_stop.1", "deck.C.remix_page"]);

    // LEDs: pad 1 bright red, page 3 on the display, slot 1 playing, sync on.
    let mut values = ValueMap::default();
    let deck = |control| ControlTarget::deck(2, control);
    values.set(deck(Control::RemixPad(1)), f32::from(led_code(1, true)));
    values.set(deck(Control::RemixPage), 3.0);
    values.set(deck(Control::RemixStop(1)), 1.0);
    values.set(deck(Control::Sync), 1.0);
    let (mut fb, mut msgs) = (rille_midi::FeedbackState::new(), Vec::new());
    fb.collect(c, &values, &mut msgs);
    let mut hid = rille_midi::hid::HidTranslator::new(std::sync::Arc::new(c.hid.clone().unwrap()));
    msgs.iter().for_each(|m| hid.output(*m));
    let mut r = Vec::new();
    hid.flush(|report| r = report.to_vec());
    let [blue, red, green] = [r[25], r[26], r[27]];
    assert!(red > 90 && green < 20 && blue < 20, "{:?}", &r[25..28]);
    assert!(r[28..73].iter().all(|&b| b == 0), "other pads off");
    let right: Vec<bool> = r[2..=8].iter().map(|&b| b > 0).collect();
    assert_eq!(right, [true, true, true, true, false, false, true], "3 = g, c, b, a, d");
    assert!(r[10..=16].iter().all(|&b| b == 0), "no leading zero");
    assert_eq!(r[73..=80], [10, 10, 10, 10, 10, 10, 127, 127], "STOP 4..1");
    assert_eq!((r[23], r[24]), (10, 127), "QUANT dim, SYNC on");
}

#[test]
fn xone_channels_follow_the_settings() {
    let (k2, x96) = ("Allen & Heath Xone:K2", "Allen & Heath Xone:96");
    let store = MappingStore::load(&[bundled_dir()]);
    let factory = [(x96.to_owned(), 16), (k2.to_owned(), 15)];
    assert_eq!(store.channels("Allen & Heath Xone:96 (CABD)"), factory);
    // A K2 on channel 14 behind the Xone:96: its notes move, the mixer's
    // faders stay on 16.
    let channels = std::collections::BTreeMap::from([(k2.to_owned(), 14)]);
    let store = MappingStore::load_with_channels(&[bundled_dir()], &channels);
    let m = store.by_name("Allen & Heath Xone:96 (CABD)").unwrap();
    assert_eq!(store.channels(&m.name), [(x96.to_owned(), 16), (k2.to_owned(), 14)]);
    let on_14 = events(m, &[[0x9d, 40, 127], [0xbf, 0, 127]]);
    assert_eq!(on_14, ["deck.C.play=down", "deck.C.volume=1.000"]);
    assert!(presses(m, &[40]).is_empty(), "channel 15 is no longer the K2's");
    let alone = store.by_name("Allen & Heath Xone:K2 (CABD)").unwrap();
    assert_eq!(events(alone, &[[0x9d, 40, 127]]), ["deck.C.play=down"]);
}
