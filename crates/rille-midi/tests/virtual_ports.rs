//! End-to-end test against a virtual ALSA sequencer device: input mapping,
//! LED feedback and hotplug. Skipped when the sequencer is unavailable.

use crossbeam_channel::{Receiver, unbounded};
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{MidiInput, MidiOutput};
use rille_core::{Control, ControlEvent, ControlTarget, ControlValue};
use rille_midi::{Mapping, MappingStore, MidiEvent, MidiManager, ValueMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

fn wait_for<T>(rx: &Receiver<T>, mut pred: impl FnMut(&T) -> bool) -> Option<T> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    while let Ok(ev) = rx.recv_deadline(deadline) {
        if pred(&ev) {
            return Some(ev);
        }
    }
    None
}

#[test]
fn virtual_device_roundtrip() {
    if !Path::new("/dev/snd/seq").exists() {
        eprintln!("ALSA sequencer unavailable, skipping");
        return;
    }
    let dev = format!("tltest{}", std::process::id());

    // The fake controller: a port we send from and one that receives LEDs.
    let mut dev_out = MidiOutput::new(&dev).unwrap().create_virtual("ctrl").unwrap();
    let (led_tx, led_rx) = unbounded();
    let _dev_in = MidiInput::new(&dev)
        .unwrap()
        .create_virtual("ctrl", move |_, bytes, _| led_tx.send(bytes.to_vec()).unwrap(), ())
        .unwrap();

    let mapping = Mapping::from_toml(&format!(
        r#"
        name = "Virtual"
        device = "^{dev}:ctrl"
        [[input]]
        target = "deck.A.play"
        midi = {{ type = "note", channel = 1, number = 11 }}
        [[output]]
        source = "deck.A.play"
        midi = {{ type = "note", channel = 1, number = 11 }}
        "#
    ))
    .unwrap();
    let mut store = MappingStore::default();
    store.push(mapping, "virtual.toml".into());

    let mut values = ValueMap::default();
    let play = ControlTarget::deck(0, Control::Play);
    values.set(play, 1.0);
    let values = Arc::new(values);
    let (tx, rx) = unbounded();
    let mut mgr = MidiManager::new("rille-test-mgr", tx, values.clone()).unwrap();
    mgr.set_raw_events(true);

    let added = mgr.refresh(&store);
    assert_eq!(added.len(), 1, "ports: {:?}", mgr.input_ports());
    let port = added[0].clone();
    assert!(wait_for(&rx, |e| matches!(e, MidiEvent::Connected { .. })).is_some());

    dev_out.send(&[0x90, 11, 127]).unwrap();
    assert!(wait_for(&rx, |e| matches!(e, MidiEvent::Raw { bytes, .. } if bytes == &[0x90, 11, 127])).is_some());
    let want = MidiEvent::Control(ControlEvent { target: play, value: ControlValue::Press(true) });
    assert!(wait_for(&rx, |e| *e == want).is_some());

    mgr.send_feedback(&*values);
    assert_eq!(wait_for(&led_rx, |_| true), Some(vec![0x90, 11, 127]));

    // Unplug: the port disappears and refresh reports it.
    drop(dev_out);
    assert!(mgr.refresh(&store).is_empty());
    let gone = MidiEvent::Disconnected { port };
    assert!(wait_for(&rx, |e| *e == gone).is_some());
    assert!(mgr.connected().is_empty());
}
