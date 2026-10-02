//! Controllers run by the kernel's snd-usb-caiaq driver, such as Native
//! Instruments' Traktor Kontrol X1 (MK1).
//!
//! These are not HID devices: the driver claims the device and offers its
//! controls as an input device (`/dev/input/event*`) and its LEDs as mixer
//! controls of a sound card. rille reads the input events and rebuilds each
//! frame of them into a report for the [`HidTranslator`](crate::hid::HidTranslator),
//! so a mapping file describes such a device with the same `[hid]` table
//! (with `transport = "caiaq"`). The rebuilt report, [`REPORT_LEN`] bytes:
//!
//! - byte 0: the layout's `input_report`
//! - bytes 1-5: buttons; key `BTN_MISC + i` is bit `i % 8` of byte
//!   `1 + i / 8`, so it becomes note `i` with `buttons = [1, 5]`
//! - bytes 6-7: four 4-bit encoder positions, as in the device's own USB
//!   packets: `ABS_X` and `ABS_Y` the low and high nibble of byte 6, `ABS_Z`
//!   and `ABS_MISC` those of byte 7
//! - bytes 8-23: the knobs `ABS_HAT0X`, `ABS_HAT0Y` … `ABS_HAT3Y`, little-endian
//!   at 8, 10 … 22
//!
//! LED messages set bytes of the layout's first output report as for a HID
//! device; each byte named in `alsa_leds` is written to that mixer control
//! (`0..=127`). Reading the input device on Linux needs the udev rule in
//! `packaging/udev/70-rille-controllers.rules`; the mixer controls need no
//! rule.

/// Length of a rebuilt report.
pub const REPORT_LEN: usize = 24;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
const BTN_MISC: u16 = 0x100;
const KEYS: u16 = 40;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT3Y: u16 = 0x17;
const ABS_MISC: u16 = 0x28;
/// Every axis the report holds.
const AXES: [u16; 12] = [ABS_X, ABS_Y, ABS_Z, ABS_MISC, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];

/// The controls' state as a report, updated one input event at a time.
#[derive(Clone, Debug)]
pub struct Report([u8; REPORT_LEN]);

impl Report {
    pub fn new(input_report: u8) -> Self {
        let mut r = [0; REPORT_LEN];
        r[0] = input_report;
        Self(r)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// Applies one key or axis event; others are ignored.
    pub fn apply(&mut self, kind: u16, code: u16, value: i32) {
        match (kind, code) {
            (EV_KEY, BTN_MISC..) if code - BTN_MISC < KEYS => {
                let i = usize::from(code - BTN_MISC);
                let (byte, mask) = (1 + i / 8, 1 << (i % 8));
                if value != 0 {
                    self.0[byte] |= mask;
                } else {
                    self.0[byte] &= !mask;
                }
            }
            (EV_ABS, ABS_X | ABS_Y | ABS_Z | ABS_MISC) => {
                let (byte, shift) = match code {
                    ABS_X => (6, 0),
                    ABS_Y => (6, 4),
                    ABS_Z => (7, 0),
                    _ => (7, 4),
                };
                let nibble = (value & 0x0F) as u8;
                self.0[byte] = (self.0[byte] & !(0x0F << shift)) | (nibble << shift);
            }
            (EV_ABS, ABS_HAT0X..=ABS_HAT3Y) => {
                let at = 8 + 2 * usize::from(code - ABS_HAT0X);
                let v = value.clamp(0, i32::from(u16::MAX)) as u16;
                self.0[at..at + 2].copy_from_slice(&v.to_le_bytes());
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{Leds, Node, Reader, scan};

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::hid::HidLayout;
    use std::ffi::CString;
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// A caiaq device's input device node and sound card.
    #[derive(Clone, Debug)]
    pub struct Node {
        pub event: PathBuf,
        pub card: Option<u32>,
        pub serial: String,
    }

    fn read_trimmed(path: &Path) -> Option<String> {
        Some(std::fs::read_to_string(path).ok()?.trim().to_owned())
    }

    fn hex_id(path: &Path) -> Option<u16> {
        u16::from_str_radix(&read_trimmed(path)?, 16).ok()
    }

    /// The sound card under a USB device's sysfs directory (the driver puts
    /// it under the device's interface).
    fn card_of(usb: &Path) -> Option<u32> {
        for intf in std::fs::read_dir(usb).ok()?.flatten() {
            let Ok(cards) = std::fs::read_dir(intf.path().join("sound")) else { continue };
            for card in cards.flatten() {
                if let Some(n) = card.file_name().to_str().and_then(|n| n.strip_prefix("card")?.parse().ok()) {
                    return Some(n);
                }
            }
        }
        None
    }

    /// The input devices plugged in now that one of `layouts` describes.
    pub fn scan(layouts: &[&Arc<HidLayout>]) -> Vec<(Node, Arc<HidLayout>)> {
        if layouts.is_empty() {
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir("/sys/class/input") else { return Vec::new() };
        let mut found = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str().filter(|n| n.starts_with("event")) else { continue };
            let input = e.path().join("device");
            let (Some(vendor), Some(product)) = (hex_id(&input.join("id/vendor")), hex_id(&input.join("id/product")))
            else {
                continue;
            };
            let Some(&layout) = layouts.iter().find(|l| l.vendor_id == vendor && l.product_id == product) else {
                continue;
            };
            // The input device's parent is the USB device.
            let usb = input.join("device");
            let node = Node {
                event: Path::new("/dev/input").join(name),
                card: card_of(&usb),
                serial: read_trimmed(&usb.join("serial")).unwrap_or_default(),
            };
            found.push((node, layout.clone()));
        }
        found.sort_by(|a, b| a.0.event.cmp(&b.0.event));
        found
    }

    /// `_IOC(_IOC_READ, 'E', nr, size)`.
    const fn eviocg(nr: u64, size: u64) -> u64 {
        (2 << 30) | (size << 16) | ((b'E' as u64) << 8) | nr
    }
    /// Bytes of the kernel's key bitmap (`KEY_MAX` = 0x2ff).
    const KEY_BYTES: usize = 0x300 / 8;

    /// Reads an input device's events and returns a report per frame.
    pub struct Reader {
        file: File,
        report: Report,
        /// Read but not yet applied.
        queue: Vec<libc::input_event>,
        next: usize,
        /// Send the report without waiting for a frame: the state at open.
        fresh: bool,
        /// Events were lost: skip to the end of the frame, then read the
        /// whole state again.
        dropped: bool,
    }

    impl Reader {
        pub fn open(node: &Node, input_report: u8) -> io::Result<Self> {
            let file = File::options().read(true).custom_flags(libc::O_NONBLOCK).open(&node.event)?;
            let mut r = Self {
                file,
                report: Report::new(input_report),
                queue: Vec::new(),
                next: 0,
                fresh: true,
                dropped: false,
            };
            r.sync()?;
            Ok(r)
        }

        /// The state of every key and axis, from the kernel.
        fn sync(&mut self) -> io::Result<()> {
            let fd = self.file.as_raw_fd();
            let mut keys = [0u8; KEY_BYTES];
            // SAFETY: EVIOCGKEY writes at most `KEY_BYTES` bytes into `keys`.
            if unsafe { libc::ioctl(fd, eviocg(0x18, KEY_BYTES as u64) as _, keys.as_mut_ptr()) } < 0 {
                return Err(io::Error::last_os_error());
            }
            for i in 0..KEYS {
                let code = usize::from(BTN_MISC + i);
                self.report.apply(EV_KEY, BTN_MISC + i, i32::from(keys[code / 8] >> (code % 8) & 1));
            }
            for axis in AXES {
                // SAFETY: EVIOCGABS fills one `input_absinfo`.
                let mut info: libc::input_absinfo = unsafe { std::mem::zeroed() };
                let size = std::mem::size_of::<libc::input_absinfo>() as u64;
                if unsafe { libc::ioctl(fd, eviocg(0x40 + u64::from(axis), size) as _, &mut info) } == 0 {
                    self.report.apply(EV_ABS, axis, info.value);
                }
            }
            Ok(())
        }

        /// Reads more events into the queue, waiting up to `timeout_ms`.
        /// False if none came.
        fn fill(&mut self, timeout_ms: i32) -> io::Result<bool> {
            let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd.
            match unsafe { libc::poll(&mut pfd, 1, timeout_ms) } {
                n if n < 0 => {
                    let e = io::Error::last_os_error();
                    return if e.kind() == io::ErrorKind::Interrupted { Ok(false) } else { Err(e) };
                }
                0 => return Ok(false),
                _ => {}
            }
            if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                return Err(io::ErrorKind::NotConnected.into());
            }
            // SAFETY: `input_event` is plain data; zeroed is a valid value.
            let mut events: [libc::input_event; 64] = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<libc::input_event>();
            // SAFETY: reads whole events into `events`.
            let n = unsafe { libc::read(pfd.fd, events.as_mut_ptr().cast(), size * events.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                return if e.kind() == io::ErrorKind::WouldBlock { Ok(false) } else { Err(e) };
            }
            self.queue.drain(..self.next);
            self.next = 0;
            self.queue.extend_from_slice(&events[..n as usize / size]);
            Ok(true)
        }

        /// The next frame's report into `buf`: 0 bytes if none came within
        /// `timeout_ms`, an error once the device is gone.
        pub fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> io::Result<usize> {
            let mut waited = false;
            loop {
                if std::mem::take(&mut self.fresh) {
                    let n = REPORT_LEN.min(buf.len());
                    buf[..n].copy_from_slice(&self.report.bytes()[..n]);
                    return Ok(n);
                }
                let Some(ev) = self.queue.get(self.next).copied() else {
                    if waited || !self.fill(timeout_ms)? {
                        return Ok(0);
                    }
                    waited = true;
                    continue;
                };
                self.next += 1;
                match (ev.type_, ev.code) {
                    (EV_SYN, SYN_DROPPED) => self.dropped = true,
                    (EV_SYN, SYN_REPORT) if self.dropped => {
                        self.dropped = false;
                        self.sync()?;
                        self.fresh = true;
                    }
                    (EV_SYN, SYN_REPORT) => self.fresh = true,
                    _ if self.dropped => {}
                    (kind, code) => self.report.apply(kind, code, ev.value),
                }
            }
        }
    }

    /// The LEDs: mixer controls of the device's sound card.
    pub struct Leds {
        ctl: alsa::ctl::Ctl,
        controls: Vec<(usize, alsa::ctl::ElemId)>,
        written: Vec<Option<u8>>,
    }

    impl Leds {
        /// `None` without a sound card.
        pub fn open(node: &Node, names: &[(usize, String)]) -> Option<Self> {
            let ctl = alsa::ctl::Ctl::new(&format!("hw:{}", node.card?), false).ok()?;
            let controls = names
                .iter()
                .filter_map(|(byte, name)| {
                    let mut id = alsa::ctl::ElemId::new(alsa::ctl::ElemIface::Mixer);
                    id.set_name(&CString::new(name.as_str()).ok()?);
                    Some((*byte, id))
                })
                .collect::<Vec<_>>();
            let written = vec![None; controls.len()];
            Some(Self { ctl, controls, written })
        }

        /// Sets every LED whose byte of `report` changed.
        pub fn write(&mut self, report: &[u8]) {
            for ((byte, id), written) in self.controls.iter().zip(&mut self.written) {
                let Some(&v) = report.get(*byte) else { continue };
                if *written == Some(v) {
                    continue;
                }
                let Ok(mut value) = alsa::ctl::ElemValue::new(alsa::ctl::ElemType::Integer) else { return };
                value.set_id(id);
                value.set_integer(0, i32::from(v.min(127)));
                // Like hidraw writes, a failure means the device is going away.
                if self.ctl.elem_write(&value).is_ok() {
                    *written = Some(v);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_become_report_bytes() {
        let mut r = Report::new(0x01);
        r.apply(EV_KEY, BTN_MISC, 1); // note 0
        r.apply(EV_KEY, BTN_MISC + 36, 1); // note 36: byte 5, bit 4
        r.apply(EV_KEY, BTN_MISC + 40, 1); // past the buttons
        r.apply(EV_ABS, ABS_Y, 0x0B);
        r.apply(EV_ABS, ABS_MISC, 0x13); // only the nibble
        r.apply(EV_ABS, ABS_HAT0X + 1, 4095); // HAT0Y
        r.apply(EV_ABS, ABS_HAT3Y, 0x0102);
        let b = r.bytes();
        assert_eq!(b[..6], [0x01, 0x01, 0, 0, 0, 0x10]);
        assert_eq!((b[6], b[7]), (0xB0, 0x30));
        assert_eq!(u16::from_le_bytes([b[10], b[11]]), 4095);
        assert_eq!((b[22], b[23]), (0x02, 0x01));
        r.apply(EV_KEY, BTN_MISC, 0);
        r.apply(EV_ABS, ABS_Y, 0);
        assert_eq!((r.bytes()[1], r.bytes()[6]), (0, 0));
    }
}
