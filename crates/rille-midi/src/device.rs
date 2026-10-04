//! Device I/O via midir (ALSA sequencer on Linux), and HID controllers via
//! [`crate::hid`], which appear as ports named `"<model> HID"`.
//!
//! Input callbacks run on midir's thread: they feed the connection's
//! [`MappingEngine`] and send [`MidiEvent`]s into the channel given to
//! [`MidiManager::new`]. LED feedback is pushed by the app calling
//! [`MidiManager::send_feedback`] at 30–60 Hz.

use crate::engine::{MappingEngine, ValueSource};
use crate::feedback::FeedbackState;
use crate::hid::{self, HidLayout, HidLink};
use crate::mapping::Mapping;
use crate::screen;
use crate::store::MappingStore;
use crossbeam_channel::Sender;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use rille_core::ControlEvent;
use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long a HID controller's displays show its decks after it connects or
/// changes deck layout.
const LAYOUT_FLASH: Duration = Duration::from_millis(1500);
/// How long a pixel display greets after connecting.
const GREETING: Duration = Duration::from_millis(1200);

#[derive(Clone, Debug, PartialEq)]
pub enum MidiEvent {
    Control(ControlEvent),
    /// Every incoming message, for MIDI learn and a monitor. Only sent while
    /// enabled with [`MidiManager::set_raw_events`].
    Raw {
        port: String,
        bytes: Vec<u8>,
        t: Instant,
    },
    Connected {
        port: String,
        mapping: Option<String>,
    },
    Disconnected {
        port: String,
    },
    /// A `deck_layout:next` input was pressed: the controller on `port` asks
    /// for its mapping's next deck layout (see
    /// [`MappingStore::next_deck_layout`]).
    NextDeckLayout {
        port: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MidiError {
    /// The MIDI system (ALSA sequencer) is unavailable.
    Init(String),
    PortNotFound(String),
    Connect(String),
}

impl fmt::Display for MidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init(e) => write!(f, "MIDI unavailable: {e}"),
            Self::PortNotFound(p) => write!(f, "MIDI port '{p}' not found"),
            Self::Connect(e) => write!(f, "cannot connect MIDI port: {e}"),
        }
    }
}

impl std::error::Error for MidiError {}

pub type SharedValues = Arc<dyn ValueSource + Send + Sync>;
type SharedEngine = Arc<Mutex<Option<MappingEngine>>>;

fn lock(e: &SharedEngine) -> MutexGuard<'_, Option<MappingEngine>> {
    e.lock().unwrap_or_else(|p| p.into_inner())
}

enum Link {
    Midi { _input: MidiInputConnection<()>, output: Option<MidiOutputConnection> },
    Hid(HidLink),
}

struct Device {
    port: String,
    engine: SharedEngine,
    link: Link,
    /// A MIDI input was seen receiving from its port (see
    /// [`MidiManager::is_alive`]); until then it is taken as alive.
    seen_subscribed: bool,
    feedback: FeedbackState,
    /// Display messages that override the feedback until the instant, see
    /// [`deck_letters`].
    flash: Option<(Instant, Vec<[u8; 3]>)>,
    /// When it connected (a pixel display greets for a moment).
    since: Instant,
}

impl Device {
    fn send(&mut self, msgs: &[[u8; 3]]) {
        match &mut self.link {
            Link::Midi { output: Some(out), .. } => {
                for m in msgs {
                    // A failed LED update is not worth reporting; the next diff retries.
                    let _ = out.send(m);
                }
            }
            Link::Midi { output: None, .. } => {}
            Link::Hid(h) => h.send(msgs),
        }
    }

    fn flash_decks(&mut self, mapping: Option<&Mapping>) {
        let msgs = mapping.map(deck_letters).unwrap_or_default();
        self.flash = (!msgs.is_empty()).then(|| (Instant::now() + LAYOUT_FLASH, msgs));
    }

    fn lights_off(&mut self) {
        let mut msgs = Vec::new();
        let mut blank = None;
        if let Some(e) = lock(&self.engine).as_ref() {
            FeedbackState::all_off(e.mapping(), &mut msgs);
            blank = e.mapping().hid.as_ref().and_then(|h| h.bitmap.as_ref()).map(|b| vec![0; b.frame_len()]);
        }
        self.send(&msgs);
        if let (Link::Hid(h), Some(frame)) = (&self.link, blank) {
            h.show(&frame);
        }
    }

    /// The frame for the pixel display of a HID controller whose mapping has
    /// one.
    fn screen_frame(&self, engine: &MappingEngine, values: &dyn ValueSource) -> Option<Vec<u8>> {
        if !matches!(self.link, Link::Hid(_)) {
            return None;
        }
        let b = engine.mapping().hid.as_ref()?.bitmap.as_ref()?;
        let frame = if self.since.elapsed() < GREETING {
            screen::message(b.width, b.height, &["RILLE", "", &engine.mapping().name])
        } else {
            match values.drum_screen() {
                Some(s) => {
                    let beat_on = values.beat_phase().rem_euclid(1.0) < 0.5;
                    screen::render(b.screen, b.width, b.height, &s, values, &|m| engine.modifier(m), beat_on)
                }
                None => screen::message(b.width, b.height, &["RILLE"]),
            }
        };
        Some(frame)
    }
}

pub struct MidiManager {
    client: String,
    tx: Sender<MidiEvent>,
    values: SharedValues,
    raw: Arc<AtomicBool>,
    scan_in: MidiInput,
    scan_out: MidiOutput,
    devices: Vec<Device>,
    buf: Vec<[u8; 3]>,
    events: Vec<ControlEvent>,
    /// Ports that failed to connect, so hotplug logs each failure once.
    failed: HashSet<String>,
    /// The HID controllers looked for, from the mappings' `[hid]` tables.
    hid_layouts: Vec<Arc<HidLayout>>,
    /// For asking the ALSA sequencer whether an input still receives.
    #[cfg(target_os = "linux")]
    seq: Option<alsa::Seq>,
}

impl MidiManager {
    /// `client` is the ALSA client name; ports of this client are hidden from
    /// the port lists so feedback can never loop back.
    pub fn new(client: &str, tx: Sender<MidiEvent>, values: SharedValues) -> Result<Self, MidiError> {
        let init = |e: midir::InitError| MidiError::Init(e.to_string());
        Ok(Self {
            client: client.to_owned(),
            tx,
            values,
            raw: Arc::new(AtomicBool::new(false)),
            scan_in: MidiInput::new(client).map_err(init)?,
            scan_out: MidiOutput::new(client).map_err(init)?,
            devices: Vec::new(),
            buf: Vec::new(),
            events: Vec::new(),
            failed: HashSet::new(),
            hid_layouts: Vec::new(),
            #[cfg(target_os = "linux")]
            seq: alsa::Seq::open(None, None, false).ok(),
        })
    }

    /// The HID controllers to look for (see
    /// [`MappingStore::hid_layouts`](crate::MappingStore::hid_layouts)).
    pub fn set_hid_layouts(&mut self, layouts: Vec<Arc<HidLayout>>) {
        self.hid_layouts = layouts;
    }

    pub fn set_raw_events(&self, on: bool) {
        self.raw.store(on, Ordering::Relaxed);
    }

    fn own(&self, name: &str) -> bool {
        name.strip_prefix(self.client.as_str()).is_some_and(|r| r.starts_with(':'))
    }

    /// MIDI input ports and supported HID controllers.
    pub fn input_ports(&self) -> Vec<String> {
        let ports = self.scan_in.ports();
        let midi = ports.iter().filter_map(|p| self.scan_in.port_name(p).ok()).filter(|n| !self.own(n));
        midi.chain(hid::scan(&self.hid_layouts).into_iter().map(|p| p.name)).collect()
    }

    pub fn output_ports(&self) -> Vec<String> {
        let ports = self.scan_out.ports();
        ports.iter().filter_map(|p| self.scan_out.port_name(p).ok()).filter(|n| !self.own(n)).collect()
    }

    /// Connected input ports with the name of their mapping.
    pub fn connected(&self) -> Vec<(String, Option<String>)> {
        self.devices
            .iter()
            .map(|d| (d.port.clone(), lock(&d.engine).as_ref().map(|e| e.mapping().name.clone())))
            .collect()
    }

    /// Feeds one incoming message of `port` to its mapping engine and sends
    /// the resulting events (and the raw message, while enabled).
    fn input_handler(&self, port: &str, engine: &SharedEngine) -> impl FnMut(&[u8]) + Send + 'static {
        let (tx, values, raw, engine, name) =
            (self.tx.clone(), self.values.clone(), self.raw.clone(), engine.clone(), port.to_owned());
        let mut events = Vec::with_capacity(8);
        move |bytes: &[u8]| {
            let t = Instant::now();
            if raw.load(Ordering::Relaxed) {
                let _ = tx.send(MidiEvent::Raw { port: name.clone(), bytes: bytes.to_vec(), t });
            }
            let mut next_layout = false;
            if let Some(e) = lock(&engine).as_mut() {
                e.handle(bytes, t, &*values, &mut events);
                next_layout = e.take_next_deck_layout();
            }
            for e in events.drain(..) {
                let _ = tx.send(MidiEvent::Control(e));
            }
            if next_layout {
                let _ = tx.send(MidiEvent::NextDeckLayout { port: name.clone() });
            }
        }
    }

    /// Opens `port` (an input port name) and the output port of the same
    /// device, if any, or the HID controller of that name. Without a mapping
    /// only raw events are produced (for MIDI learn). Connecting an already
    /// connected port swaps its mapping.
    pub fn connect(&mut self, port: &str, mapping: Option<Mapping>) -> Result<(), MidiError> {
        if self.devices.iter().any(|d| d.port == port) {
            return self.set_mapping(port, mapping);
        }
        let mapping_name = mapping.as_ref().map(|m| m.name.clone());
        let flash = mapping.as_ref().map(deck_letters).filter(|m| !m.is_empty());
        let engine: SharedEngine = Arc::new(Mutex::new(mapping.map(MappingEngine::new)));
        let link = match hid::scan(&self.hid_layouts).into_iter().find(|p| p.name == port) {
            Some(hid_port) => {
                Link::Hid(HidLink::open(&hid_port, self.input_handler(port, &engine)).map_err(MidiError::Connect)?)
            }
            None => self.connect_midi(port, &engine)?,
        };
        let flash = flash.map(|msgs| (Instant::now() + LAYOUT_FLASH, msgs));
        let device = Device {
            port: port.to_owned(),
            engine,
            link,
            seen_subscribed: false,
            feedback: FeedbackState::new(),
            flash,
            since: Instant::now(),
        };
        self.devices.push(device);
        let _ = self.tx.send(MidiEvent::Connected { port: port.to_owned(), mapping: mapping_name });
        Ok(())
    }

    fn connect_midi(&self, port: &str, engine: &SharedEngine) -> Result<Link, MidiError> {
        let mut input = MidiInput::new(&self.client).map_err(|e| MidiError::Init(e.to_string()))?;
        input.ignore(midir::Ignore::All);
        let in_port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).is_ok_and(|n| n == port))
            .ok_or_else(|| MidiError::PortNotFound(port.to_owned()))?;

        let mut handler = self.input_handler(port, engine);
        let callback = move |_stamp: u64, bytes: &[u8], _: &mut ()| handler(bytes);
        let conn = input
            .connect(&in_port, &format!("{} in", self.client), callback, ())
            .map_err(|e| MidiError::Connect(e.to_string()))?;
        Ok(Link::Midi { _input: conn, output: self.connect_output(port) })
    }

    /// The output port with the same name as the input, ignoring the ALSA
    /// `client:port` numbers (virtual devices use separate clients).
    fn connect_output(&self, port: &str) -> Option<MidiOutputConnection> {
        let out = MidiOutput::new(&self.client).ok()?;
        let names: Vec<_> = out.ports().into_iter().filter_map(|p| Some((out.port_name(&p).ok()?, p))).collect();
        let (_, p) = names
            .iter()
            .find(|(n, _)| n == port)
            .or_else(|| names.iter().find(|(n, _)| !self.own(n) && port_base_name(n) == port_base_name(port)))?;
        out.connect(p, &format!("{} out", self.client)).ok()
    }

    pub fn set_mapping(&mut self, port: &str, mapping: Option<Mapping>) -> Result<(), MidiError> {
        let d = self.devices.iter_mut().find(|d| d.port == port).ok_or_else(|| MidiError::PortNotFound(port.into()))?;
        d.lights_off();
        let name = mapping.as_ref().map(|m| m.name.clone());
        d.flash_decks(mapping.as_ref());
        let mut engine = lock(&d.engine);
        let previous = engine.take();
        *engine = mapping.map(MappingEngine::new);
        if let (Some(e), Some(previous)) = (engine.as_mut(), &previous) {
            e.keep_modifiers(previous);
        }
        drop(engine);
        d.feedback.reset();
        let _ = self.tx.send(MidiEvent::Connected { port: port.to_owned(), mapping: name });
        Ok(())
    }

    pub fn disconnect(&mut self, port: &str) {
        if let Some(i) = self.devices.iter().position(|d| d.port == port) {
            let mut d = self.devices.remove(i);
            d.lights_off();
            let _ = self.tx.send(MidiEvent::Disconnected { port: d.port.clone() });
        }
    }

    /// Hotplug: drops devices whose port vanished and connects new ports
    /// that have a mapping in `store`. Returns the newly connected ports.
    pub fn refresh(&mut self, store: &MappingStore) -> Vec<String> {
        self.refresh_with(|port, _| store.find(port).cloned())
    }

    /// Like [`refresh`](Self::refresh), with `pick` choosing the mapping for
    /// each new port, given the ports connected so far and their mappings
    /// (see [`connected`](Self::connected)); ports it returns `None` for stay
    /// disconnected.
    ///
    /// A device unplugged and plugged in again since the last call usually
    /// comes back under the same port name; its dead connection is dropped
    /// and the port connected again, with the mapping `pick` gives it.
    pub fn refresh_with(&mut self, pick: impl Fn(&str, &[(String, Option<String>)]) -> Option<Mapping>) -> Vec<String> {
        let ports = self.input_ports();
        let mut gone = Vec::new();
        for i in 0..self.devices.len() {
            if !ports.contains(&self.devices[i].port) || !self.is_alive(i) {
                gone.push(self.devices[i].port.clone());
            }
        }
        for port in gone {
            // The device is gone; dropping the connections is all we can do.
            self.devices.retain(|d| d.port != port);
            let _ = self.tx.send(MidiEvent::Disconnected { port });
        }
        self.failed.retain(|p| ports.contains(p));
        let mut added = Vec::new();
        for port in ports {
            if self.devices.iter().any(|d| d.port == port) {
                continue;
            }
            let Some(m) = pick(&port, &self.connected()) else { continue };
            match self.connect(&port, Some(m)) {
                Ok(()) => added.push(port),
                Err(e) if self.failed.insert(port.clone()) => eprintln!("{port}: {e}"),
                Err(_) => {}
            }
        }
        added
    }

    /// Whether device `i` still receives from its controller. A HID reader
    /// stops when the device goes away. A MIDI input is subscribed to the
    /// device's sequencer port, and the kernel drops that subscription when
    /// the port goes, even if a new one with the same name appears.
    fn is_alive(&mut self, i: usize) -> bool {
        let d = &mut self.devices[i];
        match &d.link {
            Link::Hid(h) => h.is_alive(),
            #[cfg(target_os = "linux")]
            Link::Midi { .. } => {
                let Some(seq) = &self.seq else { return true };
                match alsa_subscribed(seq, &d.port, &self.client) {
                    Some(true) => {
                        d.seen_subscribed = true;
                        true
                    }
                    // Only a subscription seen before counts as lost, so a
                    // port the check cannot see is never reconnected over and
                    // over.
                    Some(false) => !d.seen_subscribed,
                    None => true,
                }
            }
            #[cfg(not(target_os = "linux"))]
            Link::Midi { .. } => true,
        }
    }

    /// Sends changed LED states and flushes pending 14-bit values. Call at
    /// 30–60 Hz from the app's main loop.
    pub fn send_feedback(&mut self, values: &dyn ValueSource) {
        let now = Instant::now();
        for d in &mut self.devices {
            self.buf.clear();
            if d.flash.as_ref().is_some_and(|(until, _)| now >= *until) {
                // Show what the flash covered again.
                d.flash = None;
                d.feedback.reset();
            }
            let mut frame = None;
            if let Some(e) = lock(&d.engine).as_mut() {
                e.flush(now, values, &mut self.events);
                d.feedback.collect_with_modifiers(e.mapping(), values, &|m| e.modifier(m), &mut self.buf);
                frame = d.screen_frame(e, values);
            }
            // Outside the engine's lock: the controls keep working meanwhile.
            if let (Link::Hid(h), Some(f)) = (&d.link, &frame) {
                h.show(f);
            }
            if let Some((_, msgs)) = &d.flash {
                self.buf.extend_from_slice(msgs);
            }
            for e in self.events.drain(..) {
                let _ = self.tx.send(MidiEvent::Control(e));
            }
            d.send(&self.buf);
        }
    }
}

impl Drop for MidiManager {
    fn drop(&mut self) {
        self.devices.iter_mut().for_each(Device::lights_off);
    }
}

/// For a HID controller in a deck layout, display messages showing the deck
/// each deck section drives: its letter on that section's display. Empty
/// for other mappings.
fn deck_letters(mapping: &Mapping) -> Vec<[u8; 3]> {
    let Some(hid) = &mapping.hid else { return Vec::new() };
    let status = 0xB0 | (hid::DISPLAY_CHANNEL - 1);
    hid.displays.iter().zip(mapping.deck_order()).map(|(d, deck)| [status, d.cc, hid::DISPLAY_DECK + deck]).collect()
}

/// Whether a client named `client` receives from the sequencer port
/// `port` (a midir port name, ending in ` client:port`); `None` if the name
/// has no address.
#[cfg(target_os = "linux")]
fn alsa_subscribed(seq: &alsa::Seq, port: &str, client: &str) -> Option<bool> {
    use alsa::seq::{Addr, PortSubscribeIter, QuerySubsType};
    let (c, p) = port.rsplit_once(' ')?.1.split_once(':')?;
    let addr = Addr { client: c.parse().ok()?, port: p.parse().ok()? };
    let ours = |a: Addr| seq.get_any_client_info(a.client).is_ok_and(|i| i.get_name().is_ok_and(|n| n == client));
    Some(PortSubscribeIter::new(seq, addr, QuerySubsType::READ).any(|s| ours(s.get_dest())))
}

/// ALSA port names end in ` client:port`, which can change when the device
/// is plugged in again; strip that.
pub fn port_base_name(name: &str) -> &str {
    match name.rsplit_once(' ') {
        Some((base, id)) if id.split(':').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) => base,
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn deck_letters_label_each_section_display() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mappings/traktor-kontrol-x1-mk2.toml");
        let x1 = crate::store::load_file(&path).unwrap();
        let [ab, cd] = &x1.variants()[..] else { panic!("two layouts") };
        assert_eq!(super::deck_letters(ab), [[0xB2, 0, 100], [0xB2, 1, 101]]);
        assert_eq!(super::deck_letters(cd), [[0xB2, 0, 102], [0xB2, 1, 103]]);
        assert!(super::deck_letters(&x1).is_empty(), "no layout chosen");
    }

    #[test]
    fn base_name_strips_alsa_ids() {
        assert_eq!(super::port_base_name("DDJ-400:DDJ-400 MIDI 1 24:0"), "DDJ-400:DDJ-400 MIDI 1");
        assert_eq!(super::port_base_name("Some Port"), "Some Port");
    }
}
