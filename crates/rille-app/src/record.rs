//! Recording the main mix to a WAV file, with a cue sheet of the tracks
//! played.
//!
//! The audio thread copies every rendered block into a ring buffer (see
//! [`rille_engine::Recorder`]); a writer thread empties it into a 24-bit WAV
//! whose header is brought up to date every second, so the file stays
//! playable if the app stops unexpectedly.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use rille_engine::Recorder;

/// Seconds of audio the ring buffer holds while the writer catches up.
const BUFFER_SECS: usize = 10;

/// A recording in progress.
pub struct Recording {
    pub path: PathBuf,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    written: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    writer: Option<JoinHandle<Result<(), String>>>,
    tracks: Vec<CueTrack>,
}

/// A track in the cue sheet.
#[derive(Clone, Debug, PartialEq)]
struct CueTrack {
    secs: f64,
    performer: String,
    title: String,
}

/// What a finished recording holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Recorded {
    pub path: PathBuf,
    pub secs: f64,
    /// Frames left out because the disk did not keep up.
    pub dropped: u64,
}

impl Recording {
    /// Opens a new WAV file in `folder` named after the local time, and
    /// starts its writer. Send the returned recorder to the engine.
    pub fn start(folder: &Path, sample_rate: u32) -> io::Result<(Self, Box<Recorder>)> {
        std::fs::create_dir_all(folder)?;
        let path = free_name(folder, &format!("rille {}", local_time_name()), "wav");
        let spec =
            hound::WavSpec { channels: 2, sample_rate, bits_per_sample: 24, sample_format: hound::SampleFormat::Int };
        let wav = hound::WavWriter::create(&path, spec).map_err(io::Error::other)?;
        let (recorder, frames, dropped) = Recorder::new(sample_rate as usize * BUFFER_SECS);
        let (stop, written) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
        let (thread_stop, thread_written) = (stop.clone(), written.clone());
        let writer = std::thread::Builder::new()
            .name("recording".into())
            .spawn(move || write_wav(wav, frames, sample_rate, &thread_stop, &thread_written))?;
        let recording = Self { path, sample_rate, stop, written, dropped, writer: Some(writer), tracks: Vec::new() };
        Ok((recording, recorder))
    }

    /// Seconds recorded so far.
    pub fn secs(&self) -> f64 {
        self.written.load(Ordering::Relaxed) as f64 / f64::from(self.sample_rate)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Adds a track to the cue sheet (`<recording>.cue`), starting `secs`
    /// into the recording.
    pub fn add_track(&mut self, secs: f64, performer: &str, title: &str) {
        let secs = secs.max(0.0);
        let at = self.tracks.partition_point(|t| t.secs <= secs);
        self.tracks.insert(at, CueTrack { secs, performer: performer.to_owned(), title: title.to_owned() });
        let wav = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let title = self.path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Err(e) = std::fs::write(self.path.with_extension("cue"), cue_sheet(&title, &wav, &self.tracks)) {
            eprintln!("recording: cannot write the cue sheet: {e}");
        }
    }

    /// Waits for the writer to save what is left. Stop the engine's recorder
    /// first ([`rille_engine::Command::Record`] with `None`).
    pub fn finish(mut self) -> Result<Recorded, String> {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.writer.take().map_or(Ok(()), |w| w.join().unwrap_or_else(|_| Err("writer crashed".into())));
        result.map(|()| Recorded { path: self.path.clone(), secs: self.secs(), dropped: self.dropped() })
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn write_wav(
    mut wav: hound::WavWriter<io::BufWriter<std::fs::File>>,
    mut frames: rtrb::Consumer<[f32; 2]>,
    sample_rate: u32,
    stop: &AtomicBool,
    written: &AtomicU64,
) -> Result<(), String> {
    const FULL_SCALE: f32 = 8_388_607.0;
    let mut since_flush = 0u64;
    loop {
        // Read the stop flag first: frames pushed before it was set are
        // still taken below.
        let stopping = stop.load(Ordering::Relaxed) || frames.is_abandoned();
        let n = frames.slots();
        if n > 0 {
            let chunk = frames.read_chunk(n).map_err(|e| e.to_string())?;
            let (a, b) = chunk.as_slices();
            for f in a.iter().chain(b) {
                for s in f {
                    wav.write_sample((s.clamp(-1.0, 1.0) * FULL_SCALE).round() as i32).map_err(|e| e.to_string())?;
                }
            }
            chunk.commit_all();
            written.fetch_add(n as u64, Ordering::Relaxed);
            since_flush += n as u64;
            if since_flush >= u64::from(sample_rate) {
                since_flush = 0;
                wav.flush().map_err(|e| e.to_string())?;
            }
        } else if stopping {
            break;
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    wav.finalize().map_err(|e| e.to_string())
}

/// `<folder>/<stem>.<ext>`, or with " (2)", " (3)" … if that exists.
fn free_name(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = folder.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// The local date and time as `2026-10-02 21-30-05` (no colons, for file
/// names).
fn local_time_name() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let t = secs as libc::time_t;
    // SAFETY: `tm` is plain data; localtime_r only writes into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return secs.to_string();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// A cue sheet for `wav` (a file name next to it) with `tracks` in order.
fn cue_sheet(title: &str, wav: &str, tracks: &[CueTrack]) -> String {
    let quote = |s: &str| s.replace('"', "'");
    let mut out = format!("PERFORMER \"rille\"\nTITLE \"{}\"\nFILE \"{}\" WAVE\n", quote(title), quote(wav));
    for (i, t) in tracks.iter().enumerate() {
        // Cue sheet time is minutes:seconds:frames, 75 frames a second.
        let frames = (t.secs * 75.0).round() as u64;
        out.push_str(&format!(
            "  TRACK {:02} AUDIO\n    TITLE \"{}\"\n    PERFORMER \"{}\"\n    INDEX 01 {:02}:{:02}:{:02}\n",
            i + 1,
            quote(&t.title),
            quote(&t.performer),
            frames / 75 / 60,
            frames / 75 % 60,
            frames % 75
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_a_wav_and_a_cue_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let (mut rec, mut recorder) = Recording::start(dir.path(), 48_000).unwrap();
        let block: Vec<[f32; 2]> = (0..4800).map(|i| [(i as f32 / 4800.0) - 0.5, 2.0]).collect();
        recorder.push(&block);
        rec.add_track(62.5, "Artist \"A\"", "Second");
        rec.add_track(0.0, "Artist", "First");
        drop(recorder); // the engine let go of it
        let done = rec.finish().unwrap();
        assert_eq!((done.secs, done.dropped), (0.1, 0));
        let mut wav = hound::WavReader::open(&done.path).unwrap();
        assert_eq!((wav.spec().channels, wav.spec().sample_rate, wav.spec().bits_per_sample), (2, 48_000, 24));
        let samples: Vec<i32> = wav.samples::<i32>().map(Result::unwrap).collect();
        assert_eq!(samples.len(), 9600);
        assert_eq!((samples[0], samples[1]), (-4_194_304, 8_388_607), "-0.5, and 2.0 clipped to full scale");
        let cue = std::fs::read_to_string(done.path.with_extension("cue")).unwrap();
        let name = done.path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(cue.contains(&format!("FILE \"{name}\" WAVE")), "{cue}");
        let first = cue.find("TITLE \"First\"").unwrap();
        let second = cue.find("TITLE \"Second\"").unwrap();
        assert!(first < second, "sorted by time: {cue}");
        assert!(cue.contains("PERFORMER \"Artist 'A'\"\n    INDEX 01 01:02:38"), "{cue}");
        assert!(name.starts_with("rille 20") && name.ends_with(".wav"), "{name}");
    }

    #[test]
    fn names_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("set.wav"), "").unwrap();
        assert_eq!(free_name(dir.path(), "set", "wav"), dir.path().join("set (2).wav"));
    }
}
