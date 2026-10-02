//! Stems for the decks: the separation model (downloaded when the user asks
//! for it), the queue of tracks to separate and the cache of their stems.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rille_engine::{Command, StemAudio, TrackAudio};
use rille_library::TrackId;
use rille_stems::Separator;

use crate::{App, UiEvent};

/// Where a deck's stems are.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum DeckStems {
    #[default]
    None,
    /// Waiting for other tracks to be separated first.
    Waiting,
    /// Loading the separation model.
    LoadingModel,
    /// Separating, 0..1.
    Separating(f32),
    Ready,
    Failed(String),
}

/// The separation model's file.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelState {
    Missing,
    Downloading(f32),
    Ready,
    Failed(String),
}

struct Job {
    deck: u8,
    seq: u64,
    track_id: TrackId,
    audio: Arc<TrackAudio>,
}

#[derive(Default)]
pub(crate) struct StemsState {
    jobs: Mutex<VecDeque<Job>>,
    wake: Condvar,
    worker: AtomicBool,
    downloading: Mutex<Option<f32>>,
    download_error: Mutex<Option<String>>,
}

impl App {
    fn stem_model_path(&self) -> PathBuf {
        self.paths.data.join("models").join("htdemucs.onnx")
    }

    fn stems_dir(&self) -> PathBuf {
        self.paths.cache.join("stems")
    }

    fn stems_file(&self, id: TrackId) -> PathBuf {
        self.stems_dir().join(format!("{id}.wav"))
    }

    pub fn stem_model(&self) -> ModelState {
        if let Some(f) = *self.stems.downloading.lock().expect("stems lock") {
            return ModelState::Downloading(f);
        }
        if self.stem_model_path().exists() {
            return ModelState::Ready;
        }
        match self.stems.download_error.lock().expect("stems lock").clone() {
            Some(e) => ModelState::Failed(e),
            None => ModelState::Missing,
        }
    }

    /// Downloads the separation model in the background (see
    /// [`rille_stems::MODEL_NOTICE`]); its checksum is verified.
    pub fn download_stem_model(self: &Arc<Self>) {
        {
            let mut d = self.stems.downloading.lock().expect("stems lock");
            if d.is_some() || self.stem_model_path().exists() {
                return;
            }
            *d = Some(0.0);
        }
        *self.stems.download_error.lock().expect("stems lock") = None;
        self.notify(UiEvent::StemsChanged);
        let app = self.clone();
        std::thread::Builder::new()
            .name("stem model download".into())
            .spawn(move || {
                let dest = app.stem_model_path();
                let result = app.fetch_stem_model(&dest);
                *app.stems.downloading.lock().expect("stems lock") = None;
                match result {
                    Ok(()) => app.notify(UiEvent::Status("Stem model downloaded".into())),
                    Err(e) => {
                        app.notify(UiEvent::Status(format!("Stem model download failed: {e}")));
                        *app.stems.download_error.lock().expect("stems lock") = Some(e);
                    }
                }
                app.notify(UiEvent::StemsChanged);
            })
            .expect("spawn stem model download");
    }

    fn fetch_stem_model(&self, dest: &Path) -> Result<(), String> {
        let tmp = dest.with_extension("download");
        let cancel = AtomicBool::new(false);
        let mut last = Instant::now();
        let mut progress = |p: rille_beatport::Progress| {
            if last.elapsed() >= Duration::from_millis(250) {
                last = Instant::now();
                *self.stems.downloading.lock().expect("stems lock") = Some(p.fraction().unwrap_or(0.0));
                self.notify(UiEvent::StemsChanged);
            }
        };
        self.beatport
            .client
            .fetch_file(rille_stems::MODEL_URL, &tmp, &cancel, &mut progress, None)
            .map_err(|e| e.to_string())?;
        let sum = sha256_file(&tmp).map_err(|e| e.to_string())?;
        if sum != rille_stems::MODEL_SHA256 {
            let _ = std::fs::remove_file(&tmp);
            return Err("the downloaded model is damaged (checksum mismatch)".into());
        }
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
    }

    /// Stems for the track on `deck`: from the cache, else separated in the
    /// background (the model must be downloaded).
    pub fn prepare_stems(self: &Arc<Self>, deck: u8) {
        let info = self.deck(deck);
        let (Some(track_id), Some(audio)) = (info.track_id, info.audio.clone()) else { return };
        if info.loading || info.download.is_some() || matches!(info.stems, DeckStems::Ready) {
            return;
        }
        if self.load_cached_stems(deck, info.engine_id, track_id, &audio) {
            return;
        }
        if !self.stem_model_path().exists() {
            self.notify(UiEvent::Status("Download the stem model first (Settings → Decks & Analysis → Stems)".into()));
            return;
        }
        if matches!(info.stems, DeckStems::Waiting | DeckStems::LoadingModel | DeckStems::Separating(_)) {
            return;
        }
        self.update_deck(deck, info.engine_id, |i| i.stems = DeckStems::Waiting);
        self.stems.jobs.lock().expect("stems lock").push_back(Job { deck, seq: info.engine_id, track_id, audio });
        self.stems.wake.notify_one();
        if !self.stems.worker.swap(true, Ordering::Relaxed) {
            let app = self.clone();
            std::thread::Builder::new()
                .name("stems".into())
                .spawn(move || app.stems_worker())
                .expect("spawn stems worker");
        }
    }

    /// Loads cached stems for a deck's track; false when there are none
    /// that fit it.
    pub(crate) fn load_cached_stems(&self, deck: u8, seq: u64, id: TrackId, audio: &TrackAudio) -> bool {
        let Some(stems) = read_stems(&self.stems_file(id), audio) else { return false };
        // Recently used: last to leave the cache.
        let _ = std::fs::File::options()
            .append(true)
            .open(self.stems_file(id))
            .and_then(|f| f.set_modified(std::time::SystemTime::now()));
        if self.update_deck(deck, seq, |i| i.stems = DeckStems::Ready) {
            self.send(Command::SetStems { deck, track_id: seq, stems: Some(Arc::new(stems)) });
        }
        true
    }

    fn stems_worker(self: Arc<Self>) {
        let mut separator: Option<Separator> = None;
        loop {
            let job = {
                let mut jobs = self.stems.jobs.lock().expect("stems lock");
                loop {
                    if self.shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(j) = jobs.pop_front() {
                        break j;
                    }
                    // Nothing to do: free the model's memory until the next track.
                    separator = None;
                    jobs = self.stems.wake.wait_timeout(jobs, Duration::from_secs(1)).expect("stems lock").0;
                }
            };
            let current = |app: &App| app.deck(job.deck).engine_id == job.seq;
            if !current(&self) {
                continue;
            }
            if separator.is_none() {
                self.update_deck(job.deck, job.seq, |i| i.stems = DeckStems::LoadingModel);
                match Separator::load(&self.stem_model_path()) {
                    Ok(s) => separator = Some(s),
                    Err(e) => {
                        self.update_deck(job.deck, job.seq, |i| i.stems = DeckStems::Failed(e.clone()));
                        self.notify(UiEvent::Status(e));
                        continue;
                    }
                }
            }
            let Some(sep) = separator.as_ref() else { continue };
            let cancel = AtomicBool::new(false);
            let shown = Mutex::new(Instant::now() - Duration::from_secs(1));
            let progress = |p: f32| {
                if !current(&self) {
                    cancel.store(true, Ordering::Relaxed);
                    return;
                }
                let mut shown = shown.lock().expect("progress lock");
                if shown.elapsed() >= Duration::from_millis(250) || p >= 1.0 {
                    *shown = Instant::now();
                    self.update_deck(job.deck, job.seq, |i| i.stems = DeckStems::Separating(p));
                }
            };
            progress(0.0);
            let threads = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(1, 8);
            match sep.separate(&job.audio.frames, job.audio.sample_rate, threads, &cancel, &progress) {
                Ok(frames) => {
                    let stems = StemAudio { sample_rate: job.audio.sample_rate, frames };
                    if let Err(e) = write_stems(&self.stems_file(job.track_id), &stems) {
                        eprintln!("stems cache: {e}");
                    }
                    self.prune_stems();
                    if self.update_deck(job.deck, job.seq, |i| i.stems = DeckStems::Ready) {
                        self.send(Command::SetStems {
                            deck: job.deck,
                            track_id: job.seq,
                            stems: Some(Arc::new(stems)),
                        });
                    }
                }
                Err(_) if cancel.load(Ordering::Relaxed) => {}
                Err(e) => {
                    self.update_deck(job.deck, job.seq, |i| i.stems = DeckStems::Failed(e.clone()));
                    self.notify(UiEvent::Status(format!("Stem separation failed: {e}")));
                }
            }
        }
    }

    /// Keeps the stems cache under its size limit, least recently used first.
    fn prune_stems(&self) {
        let limit = u64::from(self.settings().stems_cache_mb) << 20;
        let Ok(dir) = std::fs::read_dir(self.stems_dir()) else { return };
        let mut files: Vec<_> = dir
            .flatten()
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        files.sort();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        for (_, len, path) in files {
            if total <= limit {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }
}

/// Stems as a 6-channel 16-bit WAV (drums, bass, vocals; left and right).
fn write_stems(path: &Path, stems: &StemAudio) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let spec = hound::WavSpec {
        channels: 6,
        sample_rate: stems.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let tmp = path.with_extension("part");
    let mut w = hound::WavWriter::create(&tmp, spec).map_err(|e| e.to_string())?;
    {
        let mut i16s = w.get_i16_writer(stems.frames.len() as u32 * 6);
        for f in &stems.frames {
            for s in f {
                i16s.write_sample(*s);
            }
        }
        i16s.flush().map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Cached stems, if they fit `audio` (same rate and length).
fn read_stems(path: &Path, audio: &TrackAudio) -> Option<StemAudio> {
    let mut r = hound::WavReader::open(path).ok()?;
    let spec = r.spec();
    let fits = spec.channels == 6
        && spec.bits_per_sample == 16
        && spec.sample_rate == audio.sample_rate
        && r.duration() as usize == audio.frames.len();
    if !fits {
        return None;
    }
    let mut frames = Vec::with_capacity(audio.frames.len());
    let mut frame = [0i16; 6];
    for (k, s) in r.samples::<i16>().enumerate() {
        frame[k % 6] = s.ok()?;
        if k % 6 == 5 {
            frames.push(frame);
        }
    }
    (frames.len() == audio.frames.len()).then_some(StemAudio { sample_rate: spec.sample_rate, frames })
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_roundtrip_through_the_cache_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("7.wav");
        let audio = TrackAudio { sample_rate: 44_100, frames: vec![[0.0; 2]; 1000] };
        let stems =
            StemAudio { sample_rate: 44_100, frames: (0..1000).map(|i| [i as i16, -(i as i16), 3, 4, 5, 6]).collect() };
        write_stems(&path, &stems).unwrap();
        let back = read_stems(&path, &audio).unwrap();
        assert_eq!(back.frames, stems.frames);
        // Stems of another length (the file changed) are not used.
        let longer = TrackAudio { sample_rate: 44_100, frames: vec![[0.0; 2]; 1001] };
        assert!(read_stems(&path, &longer).is_none());
    }

    #[test]
    fn checksum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, "abc").unwrap();
        assert_eq!(sha256_file(&path).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
