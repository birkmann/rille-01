//! HID controllers, driven as if they were MIDI devices.
//!
//! Some controllers, such as Native Instruments' Traktor Kontrol Z1, X1 MK2
//! and F1, send HID reports instead of MIDI. A mapping file describes such a
//! controller's reports in its `[hid]` table (a [`HidLayout`]). A
//! [`HidTranslator`] turns every input report into MIDI messages on
//! channel 1, so the rest of the mapping file, soft takeover and MIDI learn
//! work unchanged, and turns LED messages back into output reports:
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
//! - segment displays (layouts with [`Display`]s): CC `cc` on channel
//!   [`DISPLAY_CHANNEL`] (3) shows its value on display `cc` (see
//!   [`display_text`]): `0..=99` as a number, `100..=105` a letter A, b, C, d,
//!   E, F (deck letters), `106..=116` a loop size from 1/32 to 32 beats; 127,
//!   or anything else it cannot show, blanks it
//!
//! On layouts with RGB pads or displays, their channel is reserved for them
//! and never addresses an output report.
//!
//! Byte offsets count the report ID as byte 0, as in Mixxx's HID scripts,
//! where the bundled layouts come from. Opening a device on Linux needs read
//! and write access to its `/dev/hidraw*` node, see
//! `packaging/udev/70-rille-controllers.rules`.

#[cfg(target_os = "linux")]
use hidapi::{HidApi, HidDevice};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::sync::{Mutex, MutexGuard};

/// A HID controller's reports: the `[hid]` table of its mapping file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HidLayout {
    /// Port names are `"<name> HID"`, plus the serial number in parentheses;
    /// the mapping's `device` pattern should match them.
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// USB interface with the controls, for devices with several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<i32>,
    pub input_report: u8,
    /// Offsets of little-endian knob and fader values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knobs: Vec<usize>,
    #[serde(default = "default_knob_mask")]
    pub knob_mask: u16,
    #[serde(default = "default_knob_max")]
    pub knob_max: u16,
    /// Offset of the first button byte and the number of button bytes.
    #[serde(default)]
    pub buttons: (usize, usize),
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub encoders: Vec<Encoder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strip: Option<Strip>,
    /// Output reports as (report ID, length including the ID).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<(u8, usize)>,
    /// LEDs of the first output report lit while a button is held:
    /// (button note, LED byte, brightness when released).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held_leds: Vec<(u8, usize, u8)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<Calibration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb_pads: Option<RgbPads>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub displays: Vec<Display>,
}

fn default_knob_mask() -> u16 {
    0xFFFF
}

fn default_knob_max() -> u16 {
    4095
}

/// MIDI channel (`1..=16`, as in mapping files) of RGB pad messages.
pub const RGB_PAD_CHANNEL: u8 = 2;
/// MIDI channel (`1..=16`) of segment display messages.
pub const DISPLAY_CHANNEL: u8 = 3;
/// Bytes read per input report.
const MAX_REPORT: usize = 64;

/// A position counter of `bits` bits (4: the nibble at `shift`, 8: the whole
/// byte) in the input report; wraps around.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Encoder {
    pub offset: usize,
    #[serde(default)]
    pub shift: u8,
    pub bits: u8,
    pub cc: u8,
    pub note: u8,
}

/// Pads with an RGB LED, three bytes each in the first output report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RgbPads {
    /// First byte of pad 1; pad `n` starts at `offset + 3 * (n - 1)`.
    pub offset: usize,
    pub count: u8,
    /// The colour each of a pad's three bytes holds: 0 = red, 1 = green,
    /// 2 = blue.
    pub order: [usize; 3],
}

/// A 7-segment display. Each digit is a decimal point byte followed by the
/// segment bytes; the decimal point sits before (left of) its digit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    /// Index of the output report the digits are in.
    #[serde(default)]
    pub report: usize,
    /// The CC number (on [`DISPLAY_CHANNEL`]) that sets this display.
    #[serde(default)]
    pub cc: u8,
    /// Decimal point byte of each digit, left to right.
    pub digits: Vec<usize>,
    /// Offsets of segments a..=g from the decimal point byte.
    pub segments: [usize; 7],
    /// Brightness of a lit segment.
    pub brightness: u8,
}

/// Segments a..=g (bits 0..=6) of the digits 0..=9.
const DIGIT_SEGMENTS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
/// The letters A, b, C, d, E, F.
const LETTER_SEGMENTS: [u8; 6] = [0x77, 0x7C, 0x39, 0x5E, 0x79, 0x71];
/// Loop sizes 1/32 … 32 beats (display codes `106..=116`), as decimals so a
/// 7-segment display can show them; longer ones are cut to the digits there
/// are.
const LOOP_SIZE_TEXT: [&str; 11] = [".03125", ".0625", ".125", ".25", ".5", "1", "2", "4", "8", "16", "32"];
/// Display code of the letter A; B, C and D follow.
pub const DISPLAY_DECK: u8 = 100;
/// Display code of the first loop size.
pub const DISPLAY_LOOP_SIZE: u8 = 106;

/// What a display shows for `code`, see the module docs; empty to blank it.
pub fn display_text(code: u8) -> String {
    match code {
        0..=99 => code.to_string(),
        100..=105 => ["A", "b", "C", "d", "E", "F"][usize::from(code - 100)].to_owned(),
        106..=116 => LOOP_SIZE_TEXT[usize::from(code - DISPLAY_LOOP_SIZE)].to_owned(),
        _ => String::new(),
    }
}

/// `text` on `digits` digits, right-aligned: per digit its segments (bits
/// 0..=6 = a..=g) and whether the decimal point before it is lit. Text
/// longer than the display keeps its first digits.
fn display_glyphs(text: &str, digits: usize) -> Vec<(u8, bool)> {
    let mut glyphs: Vec<(u8, bool)> = Vec::new();
    let mut dot = false;
    for c in text.chars() {
        let segments = match c {
            '.' => {
                dot = true;
                continue;
            }
            '0'..='9' => DIGIT_SEGMENTS[c as usize - '0' as usize],
            'A' | 'a' => LETTER_SEGMENTS[0],
            'B' | 'b' => LETTER_SEGMENTS[1],
            'C' | 'c' => LETTER_SEGMENTS[2],
            'D' | 'd' => LETTER_SEGMENTS[3],
            'E' | 'e' => LETTER_SEGMENTS[4],
            'F' | 'f' => LETTER_SEGMENTS[5],
            _ => 0,
        };
        glyphs.push((segments, std::mem::take(&mut dot)));
    }
    glyphs.truncate(digits);
    let mut out = vec![(0, false); digits - glyphs.len()];
    out.extend(glyphs);
    out
}

/// A touch strip reporting up to two finger positions (11 bits each):
/// `1..=0x1C0` on the left half, `0x240..=0x400` on the right, 0 = no touch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strip {
    pub offsets: [usize; 2],
    pub cc: [u8; 2],
    pub note: [u8; 2],
}

/// A knob whose end points are stored in a feature report (little-endian
/// minimum and maximum at `offset`, report ID = byte 0).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub knob: usize,
    pub report: u8,
    pub offset: usize,
    /// Kept away from both ends so the knob reliably reaches 0 and 1.
    #[serde(default)]
    pub margin: u16,
}

impl HidLayout {
    /// Checks that every control and LED fits the reports and the MIDI
    /// messages they become.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("hid: name is empty".into());
        }
        if self.knobs.len() > 32 {
            return Err("hid: at most 32 knobs (CC 0-31 and their LSBs)".into());
        }
        if self.knob_max == 0 {
            return Err("hid: knob_max must be above 0".into());
        }
        let (first, count) = self.buttons;
        if count > 16 {
            return Err("hid: at most 16 button bytes (notes 0-127)".into());
        }
        let past = |o: usize, len: usize| o == 0 || o + len > MAX_REPORT;
        if self.knobs.iter().any(|&k| past(k, 2)) || (count > 0 && past(first, count)) {
            return Err(format!("hid: input offsets must be within bytes 1..{MAX_REPORT}"));
        }
        for e in &self.encoders {
            if !matches!((e.bits, e.shift), (4, 0 | 4) | (8, 0)) || past(e.offset, 1) {
                return Err("hid: an encoder is 4 bits (shift 0 or 4) or 8 bits (shift 0) of one input byte".into());
            }
            if e.cc > 127 || e.note > 126 {
                return Err("hid: encoder cc 0-127, note 0-126".into());
            }
        }
        if let Some(s) = &self.strip
            && (s.offsets.iter().any(|&o| past(o, 2)) || s.cc.iter().chain(&s.note).any(|&n| n > 127))
        {
            return Err("hid: strip offsets must be input bytes, cc and note 0-127".into());
        }
        if self.outputs.iter().any(|&(_, len)| len < 2) {
            return Err("hid: an output report has the ID and at least one byte".into());
        }
        let in_report = |r: usize, o: usize| self.outputs.get(r).is_some_and(|&(_, len)| o >= 1 && o < len);
        if self.held_leds.iter().any(|&(note, led, _)| note > 127 || !in_report(0, led)) {
            return Err("hid: held LEDs must be bytes of the first output report".into());
        }
        if let Some(c) = &self.calibration
            && c.knob >= self.knobs.len()
        {
            return Err("hid: calibration knob does not exist".into());
        }
        if let Some(p) = &self.rgb_pads
            && (p.count == 0
                || p.order.iter().any(|&c| c > 2)
                || !in_report(0, p.offset)
                || !in_report(0, p.offset + 3 * usize::from(p.count) - 1))
        {
            return Err("hid: RGB pads must be bytes of the first output report, order 0-2".into());
        }
        for d in &self.displays {
            let fits = d
                .digits
                .iter()
                .all(|&dp| in_report(d.report, dp) && d.segments.iter().all(|&s| in_report(d.report, dp + s)));
            if d.digits.is_empty() || !fits {
                return Err("hid: display digits must be bytes of their output report".into());
            }
        }
        Ok(())
    }
}

fn u16_at(r: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*r.get(offset)?, *r.get(offset + 1)?]))
}

const STRIP_HALF: u16 = 0x1C0;

/// Converts between one device's HID reports and MIDI messages.
#[derive(Debug)]
pub struct HidTranslator {
    layout: Arc<HidLayout>,
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
    pub fn new(layout: Arc<HidLayout>) -> Self {
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
            for &(_, led, off) in &layout.held_leds {
                if let Some(b) = first.get_mut(led) {
                    *b = off;
                }
            }
        }
        Self {
            last: None,
            ranges: vec![(0, layout.knob_max); layout.knobs.len()],
            sent: vec![None; layout.knobs.len()],
            touch: [None; 2],
            dirty: vec![true; reports.len()],
            reports,
            layout,
        }
    }

    pub fn layout(&self) -> &HidLayout {
        &self.layout
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
        let l = self.layout.clone();
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

        for e in &l.encoders {
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
        let (l, channel) = (self.layout.clone(), (msg[0] & 0x0F) + 1);
        if let Some(pads) = l.rgb_pads.as_ref().filter(|_| channel == RGB_PAD_CHANNEL) {
            self.set_pad(pads, msg[1], value);
        } else if !l.displays.is_empty() && channel == DISPLAY_CHANNEL {
            if let Some(display) = l.displays.iter().find(|d| d.cc == msg[1]) {
                self.show(display, value);
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

    /// Display `code` (see [`display_text`]), right-aligned.
    fn show(&mut self, display: &Display, code: u8) {
        let glyphs = display_glyphs(&display_text(code), display.digits.len());
        for (&dp, (bits, dot)) in display.digits.iter().zip(glyphs) {
            self.set_led(display.report, dp, if dot { display.brightness } else { 0 });
            for (s, &offset) in display.segments.iter().enumerate() {
                let lit = bits >> s & 1 != 0;
                self.set_led(display.report, dp + offset, if lit { display.brightness } else { 0 });
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
    pub layout: Arc<HidLayout>,
    #[cfg(target_os = "linux")]
    path: CString,
}

/// The HID controllers plugged in now that one of `layouts` describes (the
/// first that fits). Enumeration errors (no udev, for example) are treated as
/// no devices.
#[cfg(target_os = "linux")]
pub fn scan(layouts: &[Arc<HidLayout>]) -> Vec<HidPort> {
    if layouts.is_empty() {
        return Vec::new();
    }
    let Ok(api) = HidApi::new() else { return Vec::new() };
    let mut ports: Vec<HidPort> = Vec::new();
    for d in api.device_list() {
        let fits = |l: &&Arc<HidLayout>| {
            l.vendor_id == d.vendor_id()
                && l.product_id == d.product_id()
                && l.interface.is_none_or(|i| d.interface_number() < 0 || d.interface_number() == i)
        };
        let Some(layout) = layouts.iter().find(fits) else { continue };
        let serial = d.serial_number().unwrap_or("").trim();
        let name =
            if serial.is_empty() { format!("{} HID", layout.name) } else { format!("{} HID ({serial})", layout.name) };
        if !ports.iter().any(|p| p.name == name) {
            ports.push(HidPort { name, layout: layout.clone(), path: d.path().to_owned() });
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
    /// Cleared when reading fails: the device went away.
    alive: Arc<AtomicBool>,
}

#[cfg(target_os = "linux")]
impl HidLink {
    pub fn open(port: &HidPort, mut on_input: impl FnMut(&[u8]) + Send + 'static) -> Result<Self, String> {
        let hint = "on Linux the device may need the udev rule in packaging/udev/70-rille-controllers.rules";
        let api = HidApi::new().map_err(|e| e.to_string())?;
        let reader = api.open_path(&port.path).map_err(|e| format!("{e}; {hint}"))?;
        let writer = api.open_path(&port.path).map_err(|e| format!("{e}; {hint}"))?;
        let mut translator = HidTranslator::new(port.layout.clone());
        if let Some(c) = &port.layout.calibration {
            let mut buf = [0u8; MAX_REPORT];
            buf[0] = c.report;
            if let Ok(n) = writer.get_feature_report(&mut buf) {
                translator.calibrate(&buf[..n.min(buf.len())]);
            }
        }
        let shared = Arc::new(Mutex::new(Shared { translator, writer }));
        lock(&shared).flush();
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let (thread_shared, thread_stop, thread_alive) = (shared.clone(), stop.clone(), alive.clone());
        std::thread::Builder::new()
            .name(format!("hid {}", port.layout.name))
            .spawn(move || {
                let (mut buf, mut msgs) = ([0u8; MAX_REPORT], Vec::with_capacity(32));
                while !thread_stop.load(Ordering::Relaxed) {
                    let n = match reader.read_timeout(&mut buf, 100) {
                        Ok(0) => continue,
                        Ok(n) => n,
                        Err(_) => {
                            thread_alive.store(false, Ordering::Relaxed);
                            break;
                        }
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
        Ok(Self { shared, stop, alive })
    }

    /// False once the device went away (unplugged); it may come back under
    /// the same name, to be opened again.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
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
pub fn scan(_layouts: &[Arc<HidLayout>]) -> Vec<HidPort> {
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

    pub fn is_alive(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `[hid]` table of a bundled mapping file.
    fn bundled(file: &str) -> Arc<HidLayout> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mappings").join(file);
        let m = crate::Mapping::from_toml(&std::fs::read_to_string(path).unwrap()).unwrap();
        Arc::new(m.hid.expect("a [hid] table"))
    }

    fn z1() -> Arc<HidLayout> {
        bundled("traktor-kontrol-z1.toml")
    }

    fn x1() -> Arc<HidLayout> {
        bundled("traktor-kontrol-x1-mk2.toml")
    }

    fn f1() -> Arc<HidLayout> {
        bundled("traktor-kontrol-f1.toml")
    }

    #[test]
    fn scan_runs_without_devices() {
        // Real enumeration; must not fail on machines without the devices.
        for p in scan(&[z1(), x1(), f1()]) {
            assert!(p.name.starts_with(&p.layout.name), "{}", p.name);
        }
    }

    #[test]
    fn layouts_are_checked() {
        let mut l = (*x1()).clone();
        assert_eq!(l.validate(), Ok(()));
        l.displays[1].digits.push(0x5A);
        assert!(l.validate().is_err(), "segments past the end of report 0x81");
        let mut l = (*f1()).clone();
        l.rgb_pads.as_mut().unwrap().count = 30;
        assert!(l.validate().is_err());
        let mut l = (*z1()).clone();
        l.knobs.push(0);
        assert!(l.validate().is_err(), "byte 0 is the report ID");
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
        let mut t = HidTranslator::new(z1());
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
        let mut t = HidTranslator::new(z1());
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
        let mut t = HidTranslator::new(z1());
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
        let mut t = HidTranslator::new(x1());
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
        let mut t = HidTranslator::new(x1());
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
        assert_eq!(written.iter().map(|r| (r[0], r.len())).collect::<Vec<_>>(), [(0x80, 0x34), (0x81, 0x5B)]);
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
        let mut t = HidTranslator::new(f1());
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
        let mut t = HidTranslator::new(f1());
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
        let mut t = HidTranslator::new(f1());
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
        let mut t = HidTranslator::new(f1());
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
        assert_eq!(show(117), (blank.clone(), blank.clone()));
        // Loop size 16 beats.
        assert_eq!(show(115), (vec![0, 1, 1, 0, 0, 0, 0], vec![1, 1, 0, 1, 1, 1, 1]));
        // Other CC numbers on the display channel are ignored.
        t.output([0xB2, 1, 5]);
        let mut written = 0;
        t.flush(|_| written += 1);
        assert_eq!(written, 0);
    }

    #[test]
    fn f1_modifier_leds() {
        let mut t = HidTranslator::new(f1());
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

    #[test]
    fn display_text_and_glyphs() {
        assert_eq!(display_text(7), "7");
        assert_eq!(display_text(103), "d");
        assert_eq!(display_text(DISPLAY_LOOP_SIZE), ".03125");
        assert_eq!(display_text(DISPLAY_LOOP_SIZE + 10), "32");
        assert_eq!(display_text(127), "");
        let (one, five) = (DIGIT_SEGMENTS[1], DIGIT_SEGMENTS[5]);
        assert_eq!(display_glyphs("15", 3), [(0, false), (one, false), (five, false)]);
        // The point lights before the digit it precedes; long text is cut.
        assert_eq!(display_glyphs(".5", 3), [(0, false), (0, false), (five, true)]);
        assert_eq!(display_glyphs(".03125", 3)[0], (DIGIT_SEGMENTS[0], true));
        assert_eq!(display_glyphs(".03125", 3)[2], (one, false));
        assert_eq!(display_glyphs("", 2), [(0, false); 2]);
    }

    #[test]
    fn x1_displays_show_each_decks_loop_size() {
        let mut t = HidTranslator::new(x1());
        t.flush(|_| {});
        t.output([0xB2, 0, DISPLAY_LOOP_SIZE + 9]); // left: 16
        t.output([0xB2, 1, DISPLAY_LOOP_SIZE + 4]); // right: .5
        let mut reports = Vec::new();
        t.flush(|r| reports.push(r.to_vec()));
        assert_eq!(reports.len(), 1, "only the display report changed");
        let r = &reports[0];
        assert_eq!(r[0], 0x81);
        // Segment a of digit d (dp byte + 4), b (+3), c (+2), dp (+0).
        let (a, b, c) = (|dp: usize| r[dp + 4], |dp: usize| r[dp + 3], |dp: usize| r[dp + 2]);
        assert!(r[0x01..0x09].iter().all(|&v| v == 0), "left digit 1 blank");
        assert_eq!((a(0x09), b(0x09), c(0x09)), (0, 0x7F, 0x7F), "1");
        assert_eq!((a(0x11), b(0x11), c(0x11)), (0x7F, 0, 0x7F), "6");
        assert_eq!((r[0x29], a(0x29), b(0x29), c(0x29)), (0x7F, 0x7F, 0, 0x7F), "point and 5");
        assert!(r[0x31..].iter().all(|&v| v == 0), "strip LEDs untouched");
        t.output([0xB2, 0, 127]);
        let mut cleared = Vec::new();
        t.flush(|r| cleared = r.to_vec());
        assert!(cleared[0x01..0x19].iter().all(|&v| v == 0));
    }
}
