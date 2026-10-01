//! HID controllers, driven as if they were MIDI devices.
//!
//! Some controllers, such as Native Instruments' Traktor Kontrol Z1, X1 MK2
//! and F1, send HID reports instead of MIDI. A [`HidTranslator`] turns every
//! input report into MIDI messages on channel 1, so mapping files, soft
//! takeover and MIDI learn work unchanged, and turns LED messages back into
//! output reports:
//!
//! - knob or fader `i` → 14-bit CC `i` (MSB on `i`, LSB on `i + 32`)
//! - bit `b` of button byte `j` → note `8 * j + b`
//! - endless encoder → relative CC (two's complement), plus one press of note
//!   `note` per clockwise tick or `note + 1` per counter-clockwise tick
//! - touch strip half `s` → relative CC `cc[s]` while a finger slides, and
//!   note `note[s]` held while it is touched
//! - LEDs: note `n` with velocity `v` on channel `c` sets byte `n` of the
//!   `c`-th output report to `v` (brightness `0..=127`)
//! - RGB pads (layouts with [`RgbPads`]): note `n` with velocity `code` on
//!   channel [`RGB_PAD_CHANNEL`] (2) lights pad `n` (`1..`) in the colour of
//!   the remix LED code [`rille_core::remix::led_code`] (0 = off)
//! - segment display (layouts with [`Display`]): CC 0 on channel
//!   [`DISPLAY_CHANNEL`] (3) shows its value `0..=99`, or for `100..=105` a
//!   letter A, b, C, d, E, F (deck letters); 127, or anything else it cannot
//!   show, blanks it
//!
//! On layouts with RGB pads or a display, their channel is reserved for them
//! and never addresses an output report.
//!
//! Byte offsets count the report ID as byte 0, as in Mixxx's HID scripts,
//! where the layouts below come from.

#[cfg(target_os = "linux")]
use hidapi::{HidApi, HidDevice};
#[cfg(target_os = "linux")]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug)]
pub struct HidLayout {
    /// Port names are `"<name> HID"`, plus the serial number in parentheses.
    pub name: &'static str,
    pub vendor_id: u16,
    pub product_id: u16,
    /// USB interface with the controls, for devices with several.
    pub interface: Option<i32>,
    pub input_report: u8,
    /// Offsets of little-endian knob and fader values.
    pub knobs: &'static [usize],
    pub knob_mask: u16,
    pub knob_max: u16,
    /// Offset of the first button byte and the number of button bytes.
    pub buttons: (usize, usize),
    pub encoders: &'static [Encoder],
    pub strip: Option<Strip>,
    /// Output reports as (report ID, length including the ID).
    pub outputs: &'static [(u8, usize)],
    /// LEDs of the first output report lit while a button is held:
    /// (button note, LED byte, brightness when released).
    pub held_leds: &'static [(u8, usize, u8)],
    pub calibration: Option<Calibration>,
    pub rgb_pads: Option<RgbPads>,
    pub display: Option<Display>,
}

/// MIDI channel (`1..=16`, as in mapping files) of RGB pad messages.
pub const RGB_PAD_CHANNEL: u8 = 2;
/// MIDI channel (`1..=16`) of segment display messages.
pub const DISPLAY_CHANNEL: u8 = 3;

/// A position counter of `bits` bits (4: the nibble at `shift`, 8: the whole
/// byte) in the input report; wraps around.
#[derive(Debug)]
pub struct Encoder {
    pub offset: usize,
    pub shift: u8,
    pub bits: u8,
    pub cc: u8,
    pub note: u8,
}

/// Pads with an RGB LED, three bytes each in the first output report.
#[derive(Debug)]
pub struct RgbPads {
    /// First byte of pad 1; pad `n` starts at `offset + 3 * (n - 1)`.
    pub offset: usize,
    pub count: u8,
    /// The colour each of a pad's three bytes holds: 0 = red, 1 = green,
    /// 2 = blue.
    pub order: [usize; 3],
}

/// A two-digit 7-segment display in the first output report. Each digit is a
/// decimal point byte followed by the segment bytes.
#[derive(Debug)]
pub struct Display {
    /// Decimal point byte of the left and the right digit.
    pub digits: [usize; 2],
    /// Offsets of segments a..=g from the decimal point byte.
    pub segments: [usize; 7],
    /// Brightness of a lit segment.
    pub brightness: u8,
}

/// Segments a..=g (bits 0..=6) of the digits 0..=9.
const DIGIT_SEGMENTS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
/// The letters A, b, C, d, E, F.
const LETTER_SEGMENTS: [u8; 6] = [0x77, 0x7C, 0x39, 0x5E, 0x79, 0x71];

/// A touch strip reporting up to two finger positions (11 bits each):
/// `1..=0x1C0` on the left half, `0x240..=0x400` on the right, 0 = no touch.
#[derive(Debug)]
pub struct Strip {
    pub offsets: [usize; 2],
    pub cc: [u8; 2],
    pub note: [u8; 2],
}

/// A knob whose end points are stored in a feature report (little-endian
/// minimum and maximum at `offset`, report ID = byte 0).
#[derive(Debug)]
pub struct Calibration {
    pub knob: usize,
    pub report: u8,
    pub offset: usize,
    /// Kept away from both ends so the knob reliably reaches 0 and 1.
    pub margin: u16,
}

/// Native Instruments Traktor Kontrol Z1, from Mixxx's
/// `Traktor-Kontrol-Z1-scripts.js` (djantti).
pub static TRAKTOR_KONTROL_Z1: HidLayout = HidLayout {
    name: "Traktor Kontrol Z1",
    vendor_id: 0x17cc,
    product_id: 0x1210,
    interface: None,
    input_report: 0x01,
    // Gain, hi, mid, lo, filter of channel 1, the same of channel 2, cue mix,
    // volume 1, volume 2, crossfader.
    knobs: &[0x01, 0x03, 0x05, 0x07, 0x09, 0x0B, 0x0D, 0x0F, 0x11, 0x13, 0x15, 0x17, 0x19, 0x1B],
    knob_mask: 0xFFFF,
    knob_max: 4095,
    buttons: (0x1D, 1),
    encoders: &[],
    strip: None,
    outputs: &[(0x80, 0x16)],
    held_leds: &[(1, 0x13, 0x0A)],
    calibration: Some(Calibration { knob: 13, report: 0xD2, offset: 0x1D, margin: 5 }),
    rgb_pads: None,
    display: None,
};

/// Native Instruments Traktor Kontrol X1 MK2, from the Mixxx HID mapping by
/// infiniteloop (Traktor-Kontrol-X1-MK2-hid-scripts.js).
pub static TRAKTOR_KONTROL_X1_MK2: HidLayout = HidLayout {
    name: "Traktor Kontrol X1 MK2",
    vendor_id: 0x17cc,
    product_id: 0x1220,
    // Its only HID interface is 0 (Mixxx's file says 4; checked on a device).
    interface: None,
    input_report: 0x01,
    // FX 1 dry/wet and knobs 1-3, FX 2 dry/wet and knobs 1-3.
    knobs: &[0x01, 0x03, 0x05, 0x07, 0x09, 0x0B, 0x0D, 0x0F],
    knob_mask: 0x0FFF,
    knob_max: 4095,
    buttons: (0x13, 5),
    encoders: &[
        Encoder { offset: 0x11, shift: 4, bits: 4, cc: 64, note: 48 }, // browse
        Encoder { offset: 0x11, shift: 0, bits: 4, cc: 65, note: 50 }, // left deck
        Encoder { offset: 0x12, shift: 0, bits: 4, cc: 66, note: 52 }, // right deck
    ],
    strip: Some(Strip { offsets: [0x1B, 0x1D], cc: [70, 71], note: [56, 57] }),
    outputs: &[(0x80, 0x34)],
    held_leds: &[(10, 0x12, 0x03)],
    calibration: None,
    rgb_pads: None,
    display: None,
};

/// Native Instruments Traktor Kontrol F1, from its HID report descriptor and
/// Mixxx's `Traktor-Kontrol-F1-scripts.js` (Ilkka Tuohela).
pub static TRAKTOR_KONTROL_F1: HidLayout = HidLayout {
    name: "Traktor Kontrol F1",
    vendor_id: 0x17cc,
    product_id: 0x1120,
    interface: None,
    input_report: 0x01,
    // Knobs 1-4, faders 1-4. Mixxx found the range ends at 4092.
    knobs: &[0x06, 0x08, 0x0A, 0x0C, 0x0E, 0x10, 0x12, 0x14],
    knob_mask: 0x0FFF,
    knob_max: 4092,
    buttons: (0x01, 4),
    encoders: &[Encoder { offset: 0x05, shift: 0, bits: 8, cc: 64, note: 48 }],
    strip: None,
    outputs: &[(0x80, 0x51)],
    // BROWSE, SIZE, TYPE, REVERSE, SHIFT, CAPTURE: modifiers, lit while held.
    held_leds: &[
        (19, 0x11, 0x0A),
        (20, 0x12, 0x0A),
        (21, 0x13, 0x0A),
        (22, 0x14, 0x0A),
        (23, 0x15, 0x0A),
        (25, 0x16, 0x0A),
    ],
    calibration: None,
    // Blue, red, green per pad.
    rgb_pads: Some(RgbPads { offset: 0x19, count: 16, order: [2, 0, 1] }),
    // Decimal point, then segments g, c, b, a, f, e, d; Mixxx lights them at 0x40.
    display: Some(Display { digits: [0x09, 0x01], segments: [4, 3, 2, 7, 6, 5, 1], brightness: 0x40 }),
};

pub static LAYOUTS: &[&HidLayout] = &[&TRAKTOR_KONTROL_Z1, &TRAKTOR_KONTROL_X1_MK2, &TRAKTOR_KONTROL_F1];

fn u16_at(r: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*r.get(offset)?, *r.get(offset + 1)?]))
}

const STRIP_HALF: u16 = 0x1C0;

/// Converts between one device's HID reports and MIDI messages.
#[derive(Debug)]
pub struct HidTranslator {
    layout: &'static HidLayout,
    last: Option<Vec<u8>>,
    /// Per knob: calibrated (min, max) and the raw value last sent.
    ranges: Vec<(u16, u16)>,
    sent: Vec<Option<u16>>,
    /// Per strip finger: touched half and position.
    touch: [Option<(usize, u16)>; 2],
    reports: Vec<Vec<u8>>,
    dirty: Vec<bool>,
}

impl HidTranslator {
    pub fn new(layout: &'static HidLayout) -> Self {
        let mut reports: Vec<Vec<u8>> = layout
            .outputs
            .iter()
            .map(|&(id, len)| {
                let mut r = vec![0; len.max(1)];
                r[0] = id;
                r
            })
            .collect();
        if let Some(first) = reports.first_mut() {
            for &(_, led, off) in layout.held_leds {
                if let Some(b) = first.get_mut(led) {
                    *b = off;
                }
            }
        }
        Self {
            layout,
            last: None,
            ranges: vec![(0, layout.knob_max); layout.knobs.len()],
            sent: vec![None; layout.knobs.len()],
            touch: [None; 2],
            dirty: vec![true; reports.len()],
            reports,
        }
    }

    pub fn layout(&self) -> &'static HidLayout {
        self.layout
    }

    /// Applies the layout's calibration from its feature report (with the
    /// report ID in byte 0). Implausible values are ignored.
    pub fn calibrate(&mut self, feature_report: &[u8]) {
        let Some(c) = &self.layout.calibration else { return };
        let (Some(min), Some(max)) = (u16_at(feature_report, c.offset), u16_at(feature_report, c.offset + 2)) else {
            return;
        };
        let (min, max) = (min.saturating_add(c.margin), max.saturating_sub(c.margin));
        if min < max && max <= self.layout.knob_max && c.knob < self.ranges.len() {
            self.ranges[c.knob] = (min, max);
        }
    }

    /// Appends the MIDI messages for what changed since the previous report.
    /// The first report sends every knob and every pressed button.
    pub fn input(&mut self, r: &[u8], out: &mut Vec<[u8; 3]>) {
        let l = self.layout;
        if r.first() != Some(&l.input_report) {
            return;
        }
        let prev = self.last.take();
        let old = |i: usize| prev.as_ref().and_then(|p| p.get(i).copied());

        // Buttons first, so a modifier pressed in the same report as a knob
        // move already applies to it.
        let (first, count) = l.buttons;
        for j in 0..count {
            let Some(&now) = r.get(first + j) else { continue };
            let before = old(first + j).unwrap_or(0);
            for bit in 0..8 {
                let mask = 1 << bit;
                if (now ^ before) & mask == 0 {
                    continue;
                }
                let note = (8 * j + bit) as u8;
                let down = now & mask != 0;
                out.push(if down { [0x90, note, 127] } else { [0x80, note, 0] });
                if let Some(&(_, led, off)) = l.held_leds.iter().find(|h| h.0 == note) {
                    self.set_led(0, led, if down { 127 } else { off });
                }
            }
        }

        for (i, &off) in l.knobs.iter().enumerate() {
            let Some(raw) = u16_at(r, off) else { continue };
            let v = (raw & l.knob_mask).min(l.knob_max);
            let (lo, hi) = self.ranges[i];
            // Ignore ±1 jitter, but always reach the end points.
            let send = self.sent[i].is_none_or(|s| s.abs_diff(v) >= 2 || (s != v && (v <= lo || v >= hi)));
            if send {
                self.sent[i] = Some(v);
                let v14 = u32::from(v.clamp(lo, hi) - lo) * 16383 / u32::from(hi - lo);
                let n = i as u8;
                out.push([0xB0, n, (v14 >> 7) as u8]);
                out.push([0xB0, n + 32, (v14 & 0x7F) as u8]);
            }
        }

        for e in l.encoders {
            let (Some(&now), Some(before)) = (r.get(e.offset), old(e.offset)) else { continue };
            let range = 1i32 << e.bits;
            let pos = |b: u8| i32::from(b >> e.shift) & (range - 1);
            let ticks = (pos(now) - pos(before) + range / 2).rem_euclid(range) - range / 2;
            if ticks != 0 {
                out.push([0xB0, e.cc, (ticks.clamp(-63, 63) as i8 as u8) & 0x7F]);
                let note = if ticks > 0 { e.note } else { e.note + 1 };
                for _ in 0..ticks.abs() {
                    out.push([0x90, note, 127]);
                    out.push([0x80, note, 0]);
                }
            }
        }

        if let Some(s) = &l.strip {
            for finger in 0..2 {
                let Some(raw) = u16_at(r, s.offsets[finger]).map(|v| v & 0x07FF) else { continue };
                let now = match raw {
                    1..=STRIP_HALF => Some((0, STRIP_HALF - raw)),
                    0x240..=0x400 => Some((1, 0x400 - raw)),
                    _ => None, // released, or the centre
                };
                match (self.touch[finger], now) {
                    (Some((side, before)), Some((same, pos))) if side == same => {
                        let delta = (i32::from(pos) - i32::from(before)).clamp(-63, 63);
                        if delta != 0 {
                            out.push([0xB0, s.cc[side], (delta as i8 as u8) & 0x7F]);
                        }
                    }
                    (before, now) => {
                        if let Some((side, _)) = before {
                            out.push([0x80, s.note[side], 0]);
                        }
                        if let Some((side, _)) = now {
                            out.push([0x90, s.note[side], 127]);
                        }
                    }
                }
                self.touch[finger] = now;
            }
        }
        self.last = Some(r.to_vec());
    }

    /// Applies an LED message (note or CC; the value is the brightness, an
    /// LED code or a number, see the module docs).
    pub fn output(&mut self, msg: [u8; 3]) {
        let value = match msg[0] & 0xF0 {
            0x90 | 0xB0 => msg[2].min(127),
            0x80 => 0,
            _ => return,
        };
        let (l, channel) = (self.layout, (msg[0] & 0x0F) + 1);
        if let Some(pads) = l.rgb_pads.as_ref().filter(|_| channel == RGB_PAD_CHANNEL) {
            self.set_pad(pads, msg[1], value);
        } else if let Some(display) = l.display.as_ref().filter(|_| channel == DISPLAY_CHANNEL) {
            if msg[1] == 0 {
                self.show_number(display, value);
            }
        } else {
            self.set_led(usize::from(channel - 1), usize::from(msg[1]), value);
        }
    }

    fn set_pad(&mut self, pads: &RgbPads, pad: u8, code: u8) {
        if !(1..=pads.count).contains(&pad) {
            return;
        }
        let rgb = rille_core::remix::led_rgb(code);
        let first = pads.offset + 3 * usize::from(pad - 1);
        for (i, &c) in pads.order.iter().enumerate() {
            let v = rgb.get(c).map_or(0, |v| (v.clamp(0.0, 1.0) * 127.0).round() as u8);
            self.set_led(0, first + i, v);
        }
    }

    /// `0..=99` right-aligned without a leading zero, `100..=105` a letter
    /// A..F on the right; anything else blanks.
    fn show_number(&mut self, display: &Display, n: u8) {
        let digit = |d: u8| DIGIT_SEGMENTS[usize::from(d)];
        let (tens, ones) = match n {
            0..=9 => (0, digit(n)),
            10..=99 => (digit(n / 10), digit(n % 10)),
            100..=105 => (0, LETTER_SEGMENTS[usize::from(n - 100)]),
            _ => (0, 0),
        };
        for (&dp, bits) in display.digits.iter().zip([tens, ones]) {
            for (s, &offset) in display.segments.iter().enumerate() {
                self.set_led(0, dp + offset, if bits >> s & 1 != 0 { display.brightness } else { 0 });
            }
        }
    }

    fn set_led(&mut self, report: usize, byte: usize, value: u8) {
        // Byte 0 is the report ID.
        if byte == 0 {
            return;
        }
        if let Some(b) = self.reports.get_mut(report).and_then(|r| r.get_mut(byte))
            && *b != value
        {
            *b = value;
            self.dirty[report] = true;
        }
    }

    /// Calls `write` with every output report changed since the last flush.
    pub fn flush(&mut self, mut write: impl FnMut(&[u8])) {
        for (r, dirty) in self.reports.iter().zip(&mut self.dirty) {
            if std::mem::take(dirty) {
                write(r);
            }
        }
    }
}

/// A connected HID controller we have a layout for.
#[derive(Clone, Debug)]
pub struct HidPort {
    pub name: String,
    pub layout: &'static HidLayout,
    #[cfg(target_os = "linux")]
    path: CString,
}

/// The supported HID controllers plugged in now. Enumeration errors (no
/// udev, for example) are treated as no devices.
#[cfg(target_os = "linux")]
pub fn scan() -> Vec<HidPort> {
    let Ok(api) = HidApi::new() else { return Vec::new() };
    let mut ports: Vec<HidPort> = Vec::new();
    for d in api.device_list() {
        let fits = |l: &&&HidLayout| {
            l.vendor_id == d.vendor_id()
                && l.product_id == d.product_id()
                && l.interface.is_none_or(|i| d.interface_number() < 0 || d.interface_number() == i)
        };
        let Some(&layout) = LAYOUTS.iter().find(fits) else { continue };
        let serial = d.serial_number().unwrap_or("").trim();
        let name =
            if serial.is_empty() { format!("{} HID", layout.name) } else { format!("{} HID ({serial})", layout.name) };
        if !ports.iter().any(|p| p.name == name) {
            ports.push(HidPort { name, layout, path: d.path().to_owned() });
        }
    }
    ports
}

#[cfg(target_os = "linux")]
struct Shared {
    translator: HidTranslator,
    writer: HidDevice,
}

#[cfg(target_os = "linux")]
impl Shared {
    fn flush(&mut self) {
        let writer = &self.writer;
        // A failed LED update is not worth reporting; the device is probably
        // being unplugged and the reader thread ends.
        self.translator.flush(|r| {
            let _ = writer.write(r);
        });
    }
}

#[cfg(target_os = "linux")]
fn lock(s: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|p| p.into_inner())
}

/// An open HID controller. A thread reads its reports and passes the
/// translated MIDI messages to `on_input`; it stops when this is dropped or
/// the device goes away.
#[cfg(target_os = "linux")]
pub struct HidLink {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

#[cfg(target_os = "linux")]
impl HidLink {
    pub fn open(port: &HidPort, mut on_input: impl FnMut(&[u8]) + Send + 'static) -> Result<Self, String> {
        let hint = "on Linux the device may need the udev rule in packaging/udev/70-rille-controllers.rules";
        let api = HidApi::new().map_err(|e| e.to_string())?;
        let reader = api.open_path(&port.path).map_err(|e| format!("{e}; {hint}"))?;
        let writer = api.open_path(&port.path).map_err(|e| format!("{e}; {hint}"))?;
        let mut translator = HidTranslator::new(port.layout);
        if let Some(c) = &port.layout.calibration {
            let mut buf = [0u8; 64];
            buf[0] = c.report;
            if let Ok(n) = writer.get_feature_report(&mut buf) {
                translator.calibrate(&buf[..n.min(buf.len())]);
            }
        }
        let shared = Arc::new(Mutex::new(Shared { translator, writer }));
        lock(&shared).flush();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_shared, thread_stop) = (shared.clone(), stop.clone());
        std::thread::Builder::new()
            .name(format!("hid {}", port.layout.name))
            .spawn(move || {
                let (mut buf, mut msgs) = ([0u8; 64], Vec::with_capacity(32));
                while !thread_stop.load(Ordering::Relaxed) {
                    let n = match reader.read_timeout(&mut buf, 100) {
                        Ok(0) => continue,
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    msgs.clear();
                    {
                        let mut s = lock(&thread_shared);
                        s.translator.input(&buf[..n], &mut msgs);
                        s.flush(); // held-button LEDs
                    }
                    msgs.iter().for_each(|m| on_input(m));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { shared, stop })
    }

    /// Sends LED messages.
    pub fn send(&self, msgs: &[[u8; 3]]) {
        let mut s = lock(&self.shared);
        msgs.iter().for_each(|m| s.translator.output(*m));
        s.flush();
    }
}

#[cfg(target_os = "linux")]
impl Drop for HidLink {
    fn drop(&mut self) {
        // The reader notices within its 100 ms read timeout.
        self.stop.store(true, Ordering::Relaxed);
    }
}

// No HID device access off Linux yet: no controllers are found, and none open.

#[cfg(not(target_os = "linux"))]
pub fn scan() -> Vec<HidPort> {
    Vec::new()
}

#[cfg(not(target_os = "linux"))]
pub struct HidLink;

#[cfg(not(target_os = "linux"))]
impl HidLink {
    pub fn open(_port: &HidPort, _on_input: impl FnMut(&[u8]) + Send + 'static) -> Result<Self, String> {
        Err("HID controllers are only supported on Linux".into())
    }

    pub fn send(&self, _msgs: &[[u8; 3]]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_runs_without_devices() {
        // Real enumeration; must not fail on machines without the devices.
        for p in scan() {
            assert!(p.name.starts_with(p.layout.name), "{}", p.name);
        }
    }

    fn z1_report(knobs: [u16; 14], buttons: u8) -> Vec<u8> {
        let mut r = vec![0x01];
        knobs.iter().for_each(|k| r.extend(k.to_le_bytes()));
        r.push(buttons);
        r
    }

    fn cc14(out: &[[u8; 3]], n: u8) -> Option<u16> {
        let msb = out.iter().find(|m| m[0] == 0xB0 && m[1] == n)?;
        let lsb = out.iter().find(|m| m[0] == 0xB0 && m[1] == n + 32)?;
        Some(u16::from(msb[2]) << 7 | u16::from(lsb[2]))
    }

    #[test]
    fn z1_knobs_buttons_and_mode_led() {
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_Z1);
        let mut out = Vec::new();
        let mut knobs = [0u16; 14];
        knobs[11] = 4095; // volume 1 up
        t.input(&z1_report(knobs, 0), &mut out);
        assert_eq!(out.len(), 28, "first report sends every knob");
        assert_eq!(cc14(&out, 11), Some(16383));
        assert_eq!(cc14(&out, 0), Some(0));

        // ±1 jitter is ignored; larger moves and end points are sent.
        out.clear();
        knobs[0] = 1;
        t.input(&z1_report(knobs, 0), &mut out);
        assert!(out.is_empty());
        knobs[0] = 2048;
        knobs[11] = 4094;
        t.input(&z1_report(knobs, 0x10), &mut out);
        assert_eq!(cc14(&out, 0), Some(8193));
        assert!(cc14(&out, 11).is_none());
        assert!(out.contains(&[0x90, 4, 127]), "left CUE = bit 4 = note 4");

        // MODE (bit 1) lights its LED while held.
        let mut written = Vec::new();
        t.flush(|r| written.push(r.to_vec()));
        assert_eq!(written[0][0x13], 0x0A);
        out.clear();
        t.input(&z1_report(knobs, 0x12), &mut out);
        assert_eq!(out, [[0x90, 1, 127]]);
        written.clear();
        t.flush(|r| written.push(r.to_vec()));
        assert_eq!((written.len(), written[0][0], written[0][0x13]), (1, 0x80, 127));
        t.input(&z1_report(knobs, 0), &mut out);
        assert!(out.ends_with(&[[0x80, 1, 0], [0x80, 4, 0]]));

        // Reports with another ID are ignored.
        out.clear();
        t.input(&[0x02, 1, 2, 3], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn z1_crossfader_calibration() {
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_Z1);
        let mut feature = vec![0u8; 0x21];
        feature[0] = 0xD2;
        feature[0x1D..0x1F].copy_from_slice(&100u16.to_le_bytes());
        feature[0x1F..0x21].copy_from_slice(&4000u16.to_le_bytes());
        t.calibrate(&feature);
        let mut out = Vec::new();
        let mut knobs = [0u16; 14];
        knobs[13] = 105;
        t.input(&z1_report(knobs, 0), &mut out);
        assert_eq!(cc14(&out, 13), Some(0));
        out.clear();
        knobs[13] = 3995;
        t.input(&z1_report(knobs, 0), &mut out);
        assert_eq!(cc14(&out, 13), Some(16383));
        // Nonsense calibration is ignored.
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_Z1);
        feature[0x1F..0x21].copy_from_slice(&50u16.to_le_bytes());
        t.calibrate(&feature);
        assert_eq!(t.ranges[13], (0, 4095));
    }

    fn x1_report(f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut r = vec![0u8; 0x1F];
        r[0] = 0x01;
        f(&mut r);
        r
    }

    #[test]
    fn x1_encoders_wrap_and_send_cc_and_notes() {
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_X1_MK2);
        let mut out = Vec::new();
        t.input(&x1_report(|r| r[0x11] = 0xF5), &mut out); // browse 15, left deck 5
        out.clear();
        t.input(&x1_report(|r| r[0x11] = 0x03), &mut out); // browse +1 (wraps), left deck -2
        assert_eq!(
            out,
            [
                [0xB0, 64, 1],
                [0x90, 48, 127],
                [0x80, 48, 0],
                [0xB0, 65, 126],
                [0x90, 51, 127],
                [0x80, 51, 0],
                [0x90, 51, 127],
                [0x80, 51, 0]
            ]
        );
    }

    #[test]
    fn x1_buttons_strip_and_leds() {
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_X1_MK2);
        let mut out = Vec::new();
        t.input(&x1_report(|_| {}), &mut out);
        out.clear();
        // Left PLAY is bit 0 of byte 0x16 → note 24; SHIFT is bit 2 of 0x14 → note 10.
        t.input(&x1_report(|r| (r[0x16], r[0x14]) = (0x01, 0x04)), &mut out);
        assert_eq!(out, [[0x90, 10, 127], [0x90, 24, 127]]);

        // Strip: touch on the left half, slide, lift.
        let strip = |raw: u16| x1_report(|r| r[0x1B..0x1D].copy_from_slice(&raw.to_le_bytes()));
        out.clear();
        t.input(&strip(0x100), &mut out);
        t.input(&strip(0x0F0), &mut out);
        t.input(&strip(0), &mut out);
        assert_eq!(out, [[0x80, 10, 0], [0x80, 24, 0], [0x90, 56, 127], [0xB0, 70, 16], [0x80, 56, 0]]);

        // LED messages set bytes of report 0x80; unchanged state is not resent.
        let mut written = Vec::new();
        t.flush(|r| written.push(r.to_vec()));
        t.output([0x90, 0x31, 127]);
        t.output([0x90, 0x31, 127]);
        t.output([0x90, 0, 127]); // the report ID byte is never touched
        written.clear();
        t.flush(|r| written.push(r.to_vec()));
        assert_eq!(written.len(), 1);
        assert_eq!((written[0].len(), written[0][0], written[0][0x31]), (0x34, 0x80, 127));
        t.output([0x80, 0x31, 0]);
        written.clear();
        t.flush(|r| written.push(r.to_vec()));
        assert_eq!(written[0][0x31], 0);
    }

    fn f1_report(f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut r = vec![0u8; 22];
        r[0] = 0x01;
        f(&mut r);
        r
    }

    fn f1_leds(t: &mut HidTranslator) -> Vec<u8> {
        let mut report = Vec::new();
        t.flush(|r| report = r.to_vec());
        report
    }

    #[test]
    fn f1_buttons_pads_and_knobs() {
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_F1);
        let mut out = Vec::new();
        t.input(&f1_report(|_| {}), &mut out);
        assert_eq!(out.len(), 16, "first report sends every knob and fader");
        out.clear();
        // Pad 1 (byte 1 bit 7), pad 8 (byte 1 bit 0), pad 9 (byte 2 bit 7),
        // pad 16 (byte 2 bit 0), SHIFT (byte 3 bit 7), stop 1 (byte 4 bit 7),
        // stop 4 (byte 4 bit 4).
        t.input(&f1_report(|r| r[1..5].copy_from_slice(&[0x81, 0x81, 0x80, 0x90])), &mut out);
        let notes: Vec<u8> = out.iter().map(|m| m[1]).collect();
        assert_eq!(notes, [8 - 8, 8 - 1, 24 - 16, 24 - 9, 23, 28, 31]);
        assert!(out.iter().all(|m| m[0] == 0x90 && m[2] == 127));

        // Knob 1 at its 4092 end (upper bits ignored), fader 4 halfway.
        out.clear();
        t.input(
            &f1_report(|r| {
                r[0x06..0x08].copy_from_slice(&(0xF000 | 4092u16).to_le_bytes());
                r[0x14..0x16].copy_from_slice(&2046u16.to_le_bytes());
            }),
            &mut out,
        );
        assert_eq!(cc14(&out, 0), Some(16383));
        assert_eq!(cc14(&out, 7), Some(8191));
    }

    #[test]
    fn f1_encoder_is_a_whole_byte() {
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_F1);
        let mut out = Vec::new();
        let enc = |v: u8| f1_report(|r| r[5] = v);
        t.input(&enc(250), &mut out);
        out.clear();
        t.input(&enc(3), &mut out); // +9 across the wrap
        assert_eq!(out[0], [0xB0, 64, 9]);
        assert_eq!(out[1..].iter().filter(|m| *m == &[0x90, 48, 127]).count(), 9);
        assert_eq!(out.len(), 19);
        out.clear();
        t.input(&enc(250), &mut out); // -9 back
        assert_eq!(out[0], [0xB0, 64, 128 - 9]);
        assert_eq!(out[1..].iter().filter(|m| *m == &[0x90, 49, 127]).count(), 9);
        out.clear();
        t.input(&enc(140), &mut out); // -110
        assert_eq!(out[0], [0xB0, 64, 128 - 63], "large jumps are clamped in the CC");
    }

    #[test]
    fn f1_rgb_pads() {
        use rille_core::remix::{led_code, led_rgb};
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_F1);
        f1_leds(&mut t);
        let red = led_code(1, true);
        t.output([0x91, 1, red]);
        t.output([0x91, 16, led_code(10, false)]);
        t.output([0x91, 17, red]); // no pad 17
        t.output([0x91, 0, red]);
        let r = f1_leds(&mut t);
        let [cr, cg, cb] = led_rgb(red).map(|v| (v * 127.0).round() as u8);
        assert_eq!(r[0x19..0x1C], [cb, cr, cg], "blue, red, green");
        assert!(cr > 90 && cg < 20 && cb < 20, "{:?}", &r[0x19..0x1C]);
        let dim_blue = &r[0x19 + 45..0x19 + 48];
        assert!(dim_blue[0] > dim_blue[1] && dim_blue[0] < 40, "{dim_blue:?}");
        assert_eq!(r.len(), 0x51);
        // Only the pads changed: channel 2 never reaches bytes 0x01 or 0x11.
        assert!(r[1..0x19].iter().all(|&b| b == 0 || b == 0x0A));
        assert!(r[0x1C..0x19 + 45].iter().chain(&r[0x19 + 48..]).all(|&b| b == 0));
        t.output([0x81, 1, 0]);
        assert_eq!(f1_leds(&mut t)[0x19..0x1C], [0, 0, 0]);
    }

    #[test]
    fn f1_display_digits() {
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_F1);
        f1_leds(&mut t);
        // Segments after the decimal point in report order g, c, b, a, f, e, d,
        // compared with Mixxx's digit table.
        let mut show = |n: u8| {
            t.output([0xB2, 0, n]);
            let r = f1_leds(&mut t);
            assert_eq!((r[1], r[9]), (0, 0), "decimal points stay off");
            let lit = |dp: usize| r[dp + 1..dp + 8].iter().map(|&b| u8::from(b == 0x40)).collect::<Vec<_>>();
            (lit(9), lit(1))
        };
        let blank = vec![0; 7];
        assert_eq!(show(0), (blank.clone(), vec![0, 1, 1, 1, 1, 1, 1]));
        assert_eq!(show(7), (blank.clone(), vec![0, 1, 1, 1, 0, 0, 0]));
        assert_eq!(show(42), (vec![1, 1, 1, 0, 1, 0, 0], vec![1, 0, 1, 1, 0, 1, 1]));
        assert_eq!(show(127), (blank.clone(), blank.clone()));
        assert_eq!(show(98), (vec![1, 1, 1, 1, 1, 0, 1], vec![1; 7]));
        // Deck letters: A and d (in segment order g, c, b, a, f, e, d).
        assert_eq!(show(100), (blank.clone(), vec![1, 1, 1, 1, 1, 1, 0]));
        assert_eq!(show(103), (blank.clone(), vec![1, 1, 1, 0, 0, 1, 1]));
        assert_eq!(show(106), (blank.clone(), blank.clone()));
        // Other CC numbers on the display channel are ignored.
        t.output([0xB2, 1, 5]);
        let mut written = 0;
        t.flush(|_| written += 1);
        assert_eq!(written, 0);
    }

    #[test]
    fn f1_modifier_leds() {
        let mut t = HidTranslator::new(&TRAKTOR_KONTROL_F1);
        let r = f1_leds(&mut t);
        assert_eq!(r[0x11..=0x18], [0x0A, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A, 0, 0], "dim modifiers; QUANT, SYNC off");
        let mut out = Vec::new();
        t.input(&f1_report(|r| (r[3], r[4]) = (0x80, 0x02)), &mut out); // SHIFT, CAPTURE
        let r = f1_leds(&mut t);
        assert_eq!((r[0x15], r[0x16], r[0x11]), (127, 127, 0x0A));
        t.input(&f1_report(|_| {}), &mut out);
        assert_eq!(f1_leds(&mut t)[0x15..=0x16], [0x0A, 0x0A]);
        // SYNC follows the mapping on channel 1.
        t.output([0x90, 0x18, 127]);
        assert_eq!(f1_leds(&mut t)[0x18], 127);
    }
}
