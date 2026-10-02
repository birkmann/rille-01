//! Reading a file while it downloads.
//!
//! A download comes in several ranges at once, each written in place from
//! its start. [`Arrived`] records how far each range got, and an
//! [`ArrivedReader`] reads the partial file, waiting for bytes that have not
//! arrived yet, so a decoder can start on the first seconds of a track.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// How far a download got, shared between it and readers of the file.
#[derive(Debug, Default)]
pub struct Arrived {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct State {
    path: Option<PathBuf>,
    total: u64,
    /// Per range: its start, the first byte not written yet, and its end.
    parts: Vec<(u64, u64, u64)>,
    /// Set when the download is over.
    end: Option<Outcome>,
}

impl State {
    /// Bytes from `pos` on that can be read now.
    fn available(&self, pos: u64) -> u64 {
        if self.end == Some(Outcome::Complete) {
            return self.total.saturating_sub(pos);
        }
        self.parts
            .iter()
            .find(|(start, _, end)| (*start..*end).contains(&pos))
            .map_or(0, |&(_, next, _)| next.saturating_sub(pos))
    }
}

/// When the download is over: complete, or failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    Failed,
}

impl Arrived {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The download writes `total` bytes into `path`, as `ranges`.
    pub(crate) fn begin(&self, path: PathBuf, total: u64, ranges: &[Range<u64>]) {
        let mut s = self.lock();
        s.path = Some(path);
        s.total = total;
        s.parts = ranges.iter().map(|r| (r.start, r.start, r.end)).collect();
        self.changed.notify_all();
    }

    /// Range `part` is written up to (not including) byte `next`.
    pub(crate) fn advance(&self, part: usize, next: u64) {
        if let Some(p) = self.lock().parts.get_mut(part) {
            p.1 = next.min(p.2);
        }
        self.changed.notify_all();
    }

    pub(crate) fn finish(&self, outcome: Outcome) {
        self.lock().end = Some(outcome);
        self.changed.notify_all();
    }

    /// The file being written and its size, once the download knows them;
    /// waits up to `timeout`. `None` also when the download ended without
    /// starting (an error, or a file that cannot be read while it comes).
    pub fn started(&self, timeout: Duration) -> Option<(PathBuf, u64)> {
        let s = self.lock();
        let (s, _) = self
            .changed
            .wait_timeout_while(s, timeout, |s| s.path.is_none() && s.end.is_none())
            .unwrap_or_else(|p| p.into_inner());
        Some((s.path.clone()?, s.total))
    }

    /// `None` while the download runs.
    pub fn outcome(&self) -> Option<Outcome> {
        self.lock().end
    }

    /// Bytes from the start of the file that have all arrived.
    pub fn contiguous(&self) -> u64 {
        self.lock().available(0)
    }

    /// Opens the partial file for reading; reads past what has arrived wait
    /// for it, and fail when the download fails or `cancel` is set.
    pub fn reader(self: &Arc<Self>, cancel: Arc<AtomicBool>) -> io::Result<ArrivedReader> {
        let (path, total) = {
            let s = self.lock();
            (s.path.clone().ok_or_else(|| io::Error::other("the download has not started"))?, s.total)
        };
        Ok(ArrivedReader { arrived: self.clone(), file: File::open(path)?, pos: 0, total, cancel })
    }
}

/// Reads a file while it downloads, see [`Arrived::reader`].
pub struct ArrivedReader {
    arrived: Arc<Arrived>,
    file: File,
    pos: u64,
    total: u64,
    cancel: Arc<AtomicBool>,
}

impl ArrivedReader {
    /// The size of the whole file.
    pub fn len(&self) -> u64 {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

impl Read for ArrivedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;
        if buf.is_empty() || self.pos >= self.total {
            return Ok(0);
        }
        let mut s = self.arrived.lock();
        loop {
            let available = s.available(self.pos);
            if available > 0 {
                drop(s);
                let n = buf.len().min(usize::try_from(available).unwrap_or(usize::MAX));
                let n = self.file.read_at(&mut buf[..n], self.pos)?;
                self.pos += n as u64;
                return Ok(n);
            }
            if s.end == Some(Outcome::Failed) {
                return Err(io::Error::other("the download failed"));
            }
            if self.cancel.load(Ordering::Relaxed) {
                return Err(io::Error::other("cancelled"));
            }
            s = self.arrived.changed.wait_timeout(s, Duration::from_millis(100)).unwrap_or_else(|p| p.into_inner()).0;
        }
    }
}

impl Seek for ArrivedReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.total.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::FileExt;

    #[test]
    fn reads_wait_for_their_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.part");
        let file = File::create(&path).unwrap();
        file.set_len(8).unwrap();
        let arrived = Arrived::new();
        arrived.begin(path.clone(), 8, &[0..4, 4..8]);
        assert_eq!(arrived.started(Duration::ZERO), Some((path, 8)));
        file.write_all_at(b"ab", 0).unwrap();
        arrived.advance(0, 2);
        file.write_all_at(b"ef", 4).unwrap();
        arrived.advance(1, 6);
        assert_eq!(arrived.contiguous(), 2);

        let mut r = arrived.reader(Arc::new(AtomicBool::new(false))).unwrap();
        let mut buf = [0u8; 8];
        assert_eq!(r.read(&mut buf).unwrap(), 2, "stops where the first range ends");
        assert_eq!(&buf[..2], b"ab");
        r.seek(SeekFrom::End(-4)).unwrap();
        assert_eq!(r.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"ef");

        // A read of bytes still coming waits for them.
        r.seek(SeekFrom::Start(2)).unwrap();
        let writer = {
            let arrived = arrived.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                file.write_all_at(b"cd", 2).unwrap();
                arrived.advance(0, 4);
                file.write_all_at(b"gh", 6).unwrap();
                arrived.advance(1, 8);
                arrived.finish(Outcome::Complete);
            })
        };
        let mut all = Vec::new();
        r.read_to_end(&mut all).unwrap();
        writer.join().unwrap();
        assert_eq!(all, b"cdefgh");
        assert_eq!(arrived.contiguous(), 8);
    }

    #[test]
    fn a_failed_download_ends_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.part");
        File::create(&path).unwrap().set_len(4).unwrap();
        let arrived = Arrived::new();
        arrived.begin(path, 4, std::slice::from_ref(&(0..4)));
        let mut r = arrived.reader(Arc::new(AtomicBool::new(false))).unwrap();
        arrived.finish(Outcome::Failed);
        assert!(r.read(&mut [0u8; 4]).is_err());
        // Cancelled by the reader's side.
        let arrived = Arrived::new();
        let path = dir.path().join("u.part");
        File::create(&path).unwrap().set_len(4).unwrap();
        arrived.begin(path, 4, std::slice::from_ref(&(0..4)));
        let mut r = arrived.reader(Arc::new(AtomicBool::new(true))).unwrap();
        assert!(r.read(&mut [0u8; 4]).is_err());
        assert_eq!(Arrived::new().started(Duration::from_millis(1)), None);
    }
}
