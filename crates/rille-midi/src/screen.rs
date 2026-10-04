//! Pixel displays of HID controllers (see [`crate::hid::Bitmap`]): what they
//! show, drawn into one-bit frames.
//!
//! The drum screen ([`ScreenKind::Drum`]) shows the drum machine: pattern,
//! transport, the selected instrument and tempo, all eight instruments'
//! steps with the playhead, and what the encoder edits. While a mode button
//! is held, the steps make way for what the pads do in that mode. It reads
//! the controller's modifiers by name:
//!
//! - encoder targets (a latch group): `enc_level`, `enc_tune`, `enc_decay`
//!   (the selected instrument), `enc_volume`, `enc_filter` (the default);
//!   swing while `shift` is held
//! - held layers: `shift`, `erase`, `mute`, `solo`, `group`, `select`,
//!   `sampling`, `duplicate`, `pattern`, `scene`, `browse`, `note_repeat`,
//!   `grid`
//! - the pad mode: `play` (pads play the instruments) or step mode

use crate::engine::ValueSource;
use rille_core::drums::{INSTRUMENTS, NAMES, PATTERNS, REPEAT_NAMES, STEPS};
use rille_core::{Control, ControlTarget};
use serde::{Deserialize, Serialize};

/// What a pixel display shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenKind {
    Drum,
}

/// The drum machine as the drum screen shows it; the app fills it in, see
/// [`ValueSource::drum_screen`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrumScreen {
    pub playing: bool,
    pub record: bool,
    /// REC waits for the downbeat.
    pub counting_in: bool,
    /// Waiting for the downbeat to start; stopping at the end of the bar.
    pub waiting: bool,
    pub stopping: bool,
    /// Pattern playing and the one waiting to start (`0..16`).
    pub pattern: u8,
    pub queued: Option<u8>,
    /// Patterns with steps (bit per pattern).
    pub used: u16,
    pub selected: u8,
    /// The current pattern: steps and accents per instrument, length.
    pub steps: [u16; INSTRUMENTS],
    pub accents: [u16; INSTRUMENTS],
    pub length: u8,
    /// Step under the playhead while playing.
    pub step: Option<u8>,
    /// Bit per instrument.
    pub muted: u8,
    pub soloed: u8,
    pub repeat_rate: u8,
    pub bpm: f64,
    /// Kits by name and the one loaded.
    pub kits: Vec<String>,
    pub kit: usize,
}

/// Long instrument names, by [`NAMES`] order.
const FULL_NAMES: [&str; INSTRUMENTS] =
    ["BASS DRUM", "SNARE", "CLOSED HAT", "OPEN HAT", "CLAP", "RIM SHOT", "TOM", "CYMBAL"];

/// What SHIFT + pad does, top-left pad first (the labels printed on the
/// pads).
const SHIFT_LABELS: [&str; 16] = [
    "SEMI-", "SEMI+", "OCT-", "OCT+", "CLEAR", "CLR P", "COPY", "PASTE", "STRT", "SWG50", "NUDG<", "NUDG>", "UNDO",
    "REDO", "UNDO", "REDO",
];

/// Draws `kind` for `s` on a `width` × `height` display; `beat_on`: the
/// first half of the beat (for blinking).
pub fn render(
    kind: ScreenKind,
    width: u16,
    height: u16,
    s: &DrumScreen,
    values: &dyn ValueSource,
    modifier: &dyn Fn(&str) -> bool,
    beat_on: bool,
) -> Vec<u8> {
    let mut c = Canvas::new(usize::from(width), usize::from(height));
    match kind {
        ScreenKind::Drum => drum(&mut c, s, values, modifier, beat_on),
    }
    c.buf
}

/// A frame with `lines` of text centred, e.g. a greeting.
pub fn message(width: u16, height: u16, lines: &[&str]) -> Vec<u8> {
    let mut c = Canvas::new(usize::from(width), usize::from(height));
    let top = (c.h as i32 - 10 * lines.len() as i32) / 2;
    for (i, l) in lines.iter().enumerate() {
        let x = (c.w as i32 - text_width(l)) / 2;
        c.text(x, top + 10 * i as i32, l);
    }
    c.buf
}

fn drum(c: &mut Canvas, s: &DrumScreen, values: &dyn ValueSource, modifier: &dyn Fn(&str) -> bool, beat_on: bool) {
    let sel = usize::from(s.selected).min(INSTRUMENTS - 1);
    // Status: pattern (› the queued one), transport, REC, pad mode.
    let pat = match s.queued {
        Some(q) => format!("P{:02}>{:02}", s.pattern + 1, q + 1),
        None => format!("P{:02}", s.pattern + 1),
    };
    let mut x = c.text(0, 0, &pat) + 4;
    let transport = match (s.playing, s.waiting, s.stopping) {
        (true, true, _) => Glyph::Wait,
        (true, _, true) => Glyph::Stopping,
        (true, ..) => Glyph::Play,
        _ => Glyph::Stop,
    };
    if !(s.waiting || s.stopping) || beat_on {
        c.glyph(x, 0, transport);
    }
    x += 10;
    if s.record && (!s.counting_in || beat_on) {
        c.fill(x - 1, 0, text_width("REC") + 1, 8, true);
        c.text_color(x, 0, "REC", false);
    }
    let mode = if modifier("play") { "PADS" } else { "STEP" };
    c.text(c.w as i32 - text_width(mode), 0, mode);

    // The selected instrument, tempo.
    let name = format!("{} {}", NAMES[sel], FULL_NAMES[sel]);
    c.text(0, 9, &name);
    let bpm = format!("{:.1}", s.bpm);
    c.text(c.w as i32 - text_width(&bpm), 9, &bpm);

    let layer = [
        "shift",
        "erase",
        "mute",
        "solo",
        "group",
        "select",
        "sampling",
        "duplicate",
        "pattern",
        "scene",
        "browse",
        "note_repeat",
        "grid",
    ]
    .into_iter()
    .find(|m| modifier(m));
    let area = Area { y: 18, h: 38 };
    match layer {
        Some("shift") => labels(c, area, &SHIFT_LABELS, &[false; 16]),
        Some(m @ ("erase" | "mute" | "solo" | "group" | "select" | "sampling")) => {
            let title = match m {
                "erase" => "ERASE",
                "mute" => "MUTE",
                "solo" => "SOLO",
                "sampling" => "LOAD SAMPLE",
                _ => "SELECT",
            };
            let lit: [bool; INSTRUMENTS] = std::array::from_fn(|i| match m {
                "erase" => s.steps[i] != 0,
                "mute" => s.muted & (1 << i) != 0,
                "solo" => s.soloed & (1 << i) != 0,
                _ => i == sel,
            });
            instruments(c, area, title, &lit);
        }
        Some(m @ ("duplicate" | "pattern")) => patterns(c, area, s, m == "duplicate", beat_on),
        Some("scene" | "browse") => kits(c, area, s),
        Some("note_repeat") => repeat(c, area, s),
        Some("grid") => length(c, area, s),
        _ => grid(c, area, s, sel),
    }

    // What the encoder edits.
    encoder(c, 57, values, modifier, s);
}

/// The part of the screen between the status lines and the encoder line.
#[derive(Clone, Copy)]
struct Area {
    y: i32,
    h: i32,
}

/// All instruments' steps: a row each, the playhead inverted, the selected
/// row underlined.
fn grid(c: &mut Canvas, a: Area, s: &DrumScreen, sel: usize) {
    let row_h = a.h / INSTRUMENTS as i32;
    let len = usize::from(s.length.clamp(1, STEPS as u8));
    for (i, (&steps, &accents)) in s.steps.iter().zip(&s.accents).enumerate() {
        let y = a.y + i as i32 * row_h;
        let muted = s.muted & (1 << i) != 0 || (s.soloed != 0 && s.soloed & (1 << i) == 0);
        for step in 0..STEPS {
            let x = step as i32 * 8;
            if step >= len {
                continue;
            }
            match (steps & (1 << step) != 0, accents & (1 << step) != 0) {
                (true, true) => c.fill(x, y, 7, row_h - 1, true),
                (true, false) => c.fill(x + 1, y + 1, 5, row_h - 3, true),
                _ => c.set(x + 3, y + row_h / 2 - 1, true),
            }
            if muted {
                // Muted rows faded: every other pixel.
                for dx in 0..7 {
                    for dy in 0..row_h - 1 {
                        if (dx + dy) % 2 == 0 {
                            c.set(x + dx, y + dy, false);
                        }
                    }
                }
            }
        }
        if i == sel {
            c.fill(0, y + row_h - 1, len as i32 * 8 - 1, 1, true);
        }
    }
    if let Some(p) = s.step {
        c.invert(i32::from(p) * 8, a.y, 7, a.h);
    }
}

/// Eight instrument boxes as on the top two rows of pads.
fn instruments(c: &mut Canvas, a: Area, title: &str, lit: &[bool; INSTRUMENTS]) {
    c.text(0, a.y, title);
    let (w, h) = (c.w as i32 / 4, 13);
    for (i, &on) in lit.iter().enumerate() {
        let (x, y) = ((i % 4) as i32 * w, a.y + 11 + (i / 4) as i32 * (h + 1));
        c.frame(x, y, w - 2, h);
        if on {
            c.fill(x + 1, y + 1, w - 4, h - 2, true);
        }
        let label = format!("{} {}", char::from(b'A' + i as u8), NAMES[i]);
        c.text_color(x + (w - 2 - text_width(&label)) / 2, y + 3, &label, !on);
    }
}

/// Sixteen boxes of short labels as on the pads, top-left first.
fn labels(c: &mut Canvas, a: Area, labels: &[&str; 16], lit: &[bool; 16]) {
    let (w, h) = (c.w as i32 / 4, a.h / 4);
    for (i, label) in labels.iter().enumerate() {
        let (x, y) = ((i % 4) as i32 * w, a.y + (i / 4) as i32 * h);
        if lit[i] {
            c.fill(x, y, w - 2, h - 1, true);
        }
        c.text_color(x + (w - 2 - text_width(label)) / 2, y + (h - 8) / 2, label, !lit[i]);
    }
}

fn patterns(c: &mut Canvas, a: Area, s: &DrumScreen, copy: bool, beat_on: bool) {
    let (w, h) = (c.w as i32 / 4, a.h / 4);
    let wanted = s.queued.unwrap_or(s.pattern);
    for p in 0..PATTERNS {
        let (x, y) = ((p % 4) as i32 * w, a.y + (p / 4) as i32 * h);
        let current = p == usize::from(s.pattern);
        let lit = current || (s.queued == Some(p as u8) && beat_on);
        if lit {
            c.fill(x, y, w - 2, h - 1, true);
        } else if s.used & (1 << p) != 0 {
            c.frame(x, y, w - 2, h - 1);
        }
        let label = format!("{}{:02}", if copy && p != usize::from(wanted) { '>' } else { 'P' }, p + 1);
        c.text_color(x + (w - 2 - text_width(&label)) / 2, y + (h - 8) / 2, &label, !lit);
    }
}

fn kits(c: &mut Canvas, a: Area, s: &DrumScreen) {
    c.text(0, a.y, "KIT");
    let lines = 3usize;
    let first = s.kit.saturating_sub(1).min(s.kits.len().saturating_sub(lines));
    for (k, name) in s.kits.iter().enumerate().skip(first).take(lines) {
        let y = a.y + 10 + (k - first) as i32 * 9;
        let label = format!("{:2} {name}", k + 1);
        if k == s.kit {
            c.fill(0, y - 1, c.w as i32, 9, true);
        }
        c.text_color(2, y, &label, k != s.kit);
    }
}

fn repeat(c: &mut Canvas, a: Area, s: &DrumScreen) {
    c.text(0, a.y, "NOTE REPEAT");
    let (w, h) = (c.w as i32 / 3, 12);
    for (i, name) in REPEAT_NAMES.iter().enumerate() {
        let (x, y) = ((i % 3) as i32 * w, a.y + 12 + (i / 3) as i32 * (h + 1));
        let on = i == usize::from(s.repeat_rate);
        if on {
            c.fill(x, y, w - 2, h, true);
        }
        c.text_color(x + (w - 2 - text_width(name)) / 2, y + 2, name, !on);
    }
}

fn length(c: &mut Canvas, a: Area, s: &DrumScreen) {
    let len = s.length.clamp(1, STEPS as u8);
    c.text(0, a.y, &format!("LENGTH {len} STEPS"));
    for step in 0..STEPS as i32 {
        let (x, y) = (step * 8, a.y + 16);
        if step < i32::from(len) {
            c.fill(x, y, 7, 10, true);
        } else {
            c.frame(x, y, 7, 10);
        }
    }
}

/// The encoder's target, its value and a bar.
fn encoder(c: &mut Canvas, y: i32, values: &dyn ValueSource, modifier: &dyn Fn(&str) -> bool, s: &DrumScreen) {
    let targets = [
        ("enc_level", "LEVEL", Control::DrumSelLevel),
        ("enc_tune", "TUNE", Control::DrumSelTune),
        ("enc_decay", "DECAY", Control::DrumSelDecay),
        ("enc_volume", "VOLUME", Control::DrumLevel),
    ];
    let (label, control) = if modifier("shift") {
        ("SWING", Control::DrumSwing)
    } else {
        targets.iter().find(|t| modifier(t.0)).map_or(("FILTER", Control::DrumFilter), |t| (t.1, t.2))
    };
    let v = values.value(ControlTarget::drum(control)).clamp(0.0, 1.0);
    let text = match control {
        Control::DrumSelTune => format!("{:+.0} ST", (v - 0.5) * 24.0),
        Control::DrumSelDecay if v >= 0.999 => "FULL".to_owned(),
        Control::DrumSwing => format!("{:.0}%", 50.0 + v * 25.0),
        Control::DrumFilter if (v - 0.5).abs() < 0.02 => "OFF".to_owned(),
        Control::DrumFilter if v < 0.5 => format!("LP{:.0}", (0.5 - v) * 200.0),
        Control::DrumFilter => format!("HP{:.0}", (v - 0.5) * 200.0),
        _ => format!("{:.0}%", v * 100.0),
    };
    let label = match control {
        Control::DrumSelLevel | Control::DrumSelTune | Control::DrumSelDecay => {
            format!("{} {}", NAMES[usize::from(s.selected).min(INSTRUMENTS - 1)], label)
        }
        _ => label.to_owned(),
    };
    c.text(0, y, &label);
    let tx = c.w as i32 - text_width(&text);
    c.text(tx, y, &text);
    let (bx, bw) = (text_width(&label) + 4, tx - text_width(&label) - 8);
    if bw > 8 {
        c.frame(bx, y + 1, bw, 5);
        let bipolar = matches!(control, Control::DrumFilter | Control::DrumSelTune);
        let inner = bw - 2;
        let at = (v * inner as f32).round() as i32;
        if bipolar {
            let mid = inner / 2;
            let (from, to) = if at < mid { (at, mid) } else { (mid, at) };
            c.fill(bx + 1 + from, y + 2, (to - from).max(1), 3, true);
        } else {
            c.fill(bx + 1, y + 2, at, 3, true);
        }
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Play,
    Stop,
    Wait,
    Stopping,
}

/// A one-bit frame in the display's byte order (columns of eight pixels per
/// page, bit 0 at the top).
struct Canvas {
    w: usize,
    h: usize,
    buf: Vec<u8>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self { w, h, buf: vec![0; w * h.div_ceil(8)] }
    }

    fn set(&mut self, x: i32, y: i32, on: bool) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        let i = y / 8 * self.w + x;
        if on {
            self.buf[i] |= 1 << (y % 8);
        } else {
            self.buf[i] &= !(1 << (y % 8));
        }
    }

    fn get(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return false;
        }
        let (x, y) = (x as usize, y as usize);
        self.buf[y / 8 * self.w + x] & (1 << (y % 8)) != 0
    }

    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, on: bool) {
        for dy in 0..h {
            for dx in 0..w {
                self.set(x + dx, y + dy, on);
            }
        }
    }

    fn invert(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for dy in 0..h {
            for dx in 0..w {
                let on = self.get(x + dx, y + dy);
                self.set(x + dx, y + dy, !on);
            }
        }
    }

    fn frame(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.fill(x, y, w, 1, true);
        self.fill(x, y + h - 1, w, 1, true);
        self.fill(x, y, 1, h, true);
        self.fill(x + w - 1, y, 1, h, true);
    }

    /// Draws `s` with its top left at (`x`, `y`); returns where it ends.
    fn text(&mut self, x: i32, y: i32, s: &str) -> i32 {
        self.text_color(x, y, s, true)
    }

    fn text_color(&mut self, mut x: i32, y: i32, s: &str, on: bool) -> i32 {
        for ch in s.chars() {
            let cols = glyph_columns(ch);
            for (dx, col) in cols.iter().enumerate() {
                for dy in 0..7 {
                    if col & (1 << dy) != 0 {
                        self.set(x + dx as i32, y + dy, on);
                    }
                }
            }
            x += 6;
        }
        x
    }

    fn glyph(&mut self, x: i32, y: i32, g: Glyph) {
        match g {
            Glyph::Play => {
                for dx in 0..4 {
                    self.fill(x + dx, y + dx, 1, 7 - 2 * dx, true);
                }
            }
            Glyph::Stop => self.fill(x, y + 1, 6, 6, true),
            Glyph::Wait => {
                self.frame(x, y, 7, 7);
                self.fill(x + 3, y + 1, 1, 3, true);
                self.fill(x + 4, y + 3, 2, 1, true);
            }
            Glyph::Stopping => {
                self.frame(x, y + 1, 6, 6);
            }
        }
    }
}

fn text_width(s: &str) -> i32 {
    6 * s.chars().count() as i32 - 1
}

/// Columns of a 5×7 glyph (bit 0 at the top); lower case is drawn upper
/// case, anything else outside ASCII as a box.
fn glyph_columns(ch: char) -> [u8; 5] {
    let ch = ch.to_ascii_uppercase();
    match ch {
        ' '..='`' => FONT[ch as usize - ' ' as usize],
        '{' => [0x00, 0x08, 0x36, 0x41, 0x00],
        '|' => [0x00, 0x00, 0x7F, 0x00, 0x00],
        '}' => [0x00, 0x41, 0x36, 0x08, 0x00],
        _ => [0x7F, 0x41, 0x41, 0x41, 0x7F],
    }
}

/// ASCII ' ' to '`'.
const FONT: [[u8; 5]; 65] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // ' '
    [0x00, 0x00, 0x5F, 0x00, 0x00], // !
    [0x00, 0x07, 0x00, 0x07, 0x00], // "
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // #
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // $
    [0x23, 0x13, 0x08, 0x64, 0x62], // %
    [0x36, 0x49, 0x56, 0x20, 0x50], // &
    [0x00, 0x05, 0x03, 0x00, 0x00], // '
    [0x00, 0x1C, 0x22, 0x41, 0x00], // (
    [0x00, 0x41, 0x22, 0x1C, 0x00], // )
    [0x2A, 0x1C, 0x7F, 0x1C, 0x2A], // *
    [0x08, 0x08, 0x3E, 0x08, 0x08], // +
    [0x00, 0x50, 0x30, 0x00, 0x00], // ,
    [0x08, 0x08, 0x08, 0x08, 0x08], // -
    [0x00, 0x60, 0x60, 0x00, 0x00], // .
    [0x20, 0x10, 0x08, 0x04, 0x02], // /
    [0x3E, 0x51, 0x49, 0x45, 0x3E], // 0
    [0x00, 0x42, 0x7F, 0x40, 0x00], // 1
    [0x42, 0x61, 0x51, 0x49, 0x46], // 2
    [0x21, 0x41, 0x45, 0x4B, 0x31], // 3
    [0x18, 0x14, 0x12, 0x7F, 0x10], // 4
    [0x27, 0x45, 0x45, 0x45, 0x39], // 5
    [0x3C, 0x4A, 0x49, 0x49, 0x30], // 6
    [0x01, 0x71, 0x09, 0x05, 0x03], // 7
    [0x36, 0x49, 0x49, 0x49, 0x36], // 8
    [0x06, 0x49, 0x49, 0x29, 0x1E], // 9
    [0x00, 0x36, 0x36, 0x00, 0x00], // :
    [0x00, 0x56, 0x36, 0x00, 0x00], // ;
    [0x08, 0x14, 0x22, 0x41, 0x00], // <
    [0x14, 0x14, 0x14, 0x14, 0x14], // =
    [0x00, 0x41, 0x22, 0x14, 0x08], // >
    [0x02, 0x01, 0x51, 0x09, 0x06], // ?
    [0x32, 0x49, 0x79, 0x41, 0x3E], // @
    [0x7E, 0x11, 0x11, 0x11, 0x7E], // A
    [0x7F, 0x49, 0x49, 0x49, 0x36], // B
    [0x3E, 0x41, 0x41, 0x41, 0x22], // C
    [0x7F, 0x41, 0x41, 0x22, 0x1C], // D
    [0x7F, 0x49, 0x49, 0x49, 0x41], // E
    [0x7F, 0x09, 0x09, 0x09, 0x01], // F
    [0x3E, 0x41, 0x49, 0x49, 0x7A], // G
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // H
    [0x00, 0x41, 0x7F, 0x41, 0x00], // I
    [0x20, 0x40, 0x41, 0x3F, 0x01], // J
    [0x7F, 0x08, 0x14, 0x22, 0x41], // K
    [0x7F, 0x40, 0x40, 0x40, 0x40], // L
    [0x7F, 0x02, 0x0C, 0x02, 0x7F], // M
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // N
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // O
    [0x7F, 0x09, 0x09, 0x09, 0x06], // P
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // Q
    [0x7F, 0x09, 0x19, 0x29, 0x46], // R
    [0x46, 0x49, 0x49, 0x49, 0x31], // S
    [0x01, 0x01, 0x7F, 0x01, 0x01], // T
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // U
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // V
    [0x3F, 0x40, 0x38, 0x40, 0x3F], // W
    [0x63, 0x14, 0x08, 0x14, 0x63], // X
    [0x07, 0x08, 0x70, 0x08, 0x07], // Y
    [0x61, 0x51, 0x49, 0x45, 0x43], // Z
    [0x00, 0x7F, 0x41, 0x41, 0x00], // [
    [0x02, 0x04, 0x08, 0x10, 0x20], // backslash
    [0x00, 0x41, 0x41, 0x7F, 0x00], // ]
    [0x04, 0x02, 0x01, 0x02, 0x04], // ^
    [0x40, 0x40, 0x40, 0x40, 0x40], // _
    [0x00, 0x01, 0x02, 0x04, 0x00], // `
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ValueMap;

    fn lit(frame: &[u8]) -> usize {
        frame.iter().map(|b| b.count_ones() as usize).sum()
    }

    fn screen() -> DrumScreen {
        let mut s = DrumScreen { length: 16, bpm: 124.0, kits: vec!["909".into(), "808".into()], ..Default::default() };
        s.steps[0] = 0x1111;
        s.accents[0] = 0x0001;
        s
    }

    #[test]
    fn frames_have_the_display_size_and_byte_order() {
        let f = render(ScreenKind::Drum, 128, 64, &screen(), &ValueMap::default(), &|_| false, true);
        assert_eq!(f.len(), 128 * 8);
        let mut c = Canvas::new(128, 64);
        c.set(3, 10, true);
        assert_eq!(c.buf[128 + 3], 1 << 2, "page 1, column 3, bit 2");
    }

    #[test]
    fn steps_and_playhead_are_drawn() {
        let values = ValueMap::default();
        let s = screen();
        let f = render(ScreenKind::Drum, 128, 64, &s, &values, &|_| false, true);
        let mut playing = s.clone();
        playing.playing = true;
        playing.step = Some(2);
        let g = render(ScreenKind::Drum, 128, 64, &playing, &values, &|_| false, true);
        assert_ne!(f, g, "the playhead column is inverted");
        let mut more = s.clone();
        more.steps[3] = 0xFFFF;
        assert!(lit(&render(ScreenKind::Drum, 128, 64, &more, &values, &|_| false, true)) > lit(&f));
    }

    #[test]
    fn held_modes_replace_the_steps() {
        let values = ValueMap::default();
        let s = screen();
        let base = render(ScreenKind::Drum, 128, 64, &s, &values, &|_| false, true);
        for m in ["pattern", "mute", "shift", "note_repeat", "grid", "scene"] {
            let f = render(ScreenKind::Drum, 128, 64, &s, &values, &|n| n == m, true);
            assert_ne!(f, base, "{m}");
        }
    }

    #[test]
    fn font_covers_printable_ascii() {
        assert_eq!(FONT.len(), '`' as usize - ' ' as usize + 1);
        assert_eq!(glyph_columns('a'), glyph_columns('A'));
        assert_eq!(text_width("ABC"), 17);
    }
}
