//! Tags and covers for explorer folders, read in the background so a folder
//! (a USB stick) shows its metadata without being imported. Three passes,
//! so BPMs are there to sort by early: a Serato database on the drive, then
//! the tags without covers (a few milliseconds a file), then everything
//! (tens of milliseconds a file on a stick). The first two are kept in
//! memory, the last goes to the library's `file_meta` cache; the browser
//! re-reads the folder when a batch arrives.

use crate::{App, UiEvent};
use rille_library::{FileMeta, TrackRow};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Files read in parallel: USB sticks are slow at random reads.
const THREADS: usize = 4;
/// Tag-only reads mostly wait on the drive, so more of them at once.
const QUICK_THREADS: usize = 8;
/// Files read before the cache is written; the first batch is small so the
/// visible rows fill in quickly.
const FIRST_BATCH: usize = 16;
const BATCH: usize = 64;
/// How often the browser is told to re-read the folder while reading.
const NOTIFY_EVERY: Duration = Duration::from_millis(400);

/// A drive's Serato tracks, with the database's modification time.
type SeratoDb = (Option<std::time::SystemTime>, Arc<HashMap<PathBuf, FileMeta>>);

#[derive(Default)]
pub(crate) struct FolderMetaState {
    inner: Mutex<Inner>,
    wake: Condvar,
}

#[derive(Default)]
struct Inner {
    /// The folder to read and its files without cached metadata.
    job: Option<(PathBuf, Vec<PathBuf>)>,
    /// The folder being read, if any.
    running: Option<PathBuf>,
    /// Bumped by every new job: the worker drops the folder it was reading.
    generation: u64,
    /// Files that could not be read this session; not tried again.
    failed: HashSet<PathBuf>,
    /// Serato data and tags without covers, until the file's full read is
    /// cached.
    early: HashMap<PathBuf, FileMeta>,
    started: bool,
}

impl App {
    /// Queues `files` of `dir` (canonical paths without cached metadata) for
    /// reading. A request for the folder already being read is ignored; any
    /// other replaces what was being read.
    pub(crate) fn request_folder_meta(self: &Arc<Self>, dir: PathBuf, files: Vec<PathBuf>) {
        let mut inner = self.folder_meta.inner.lock().expect("folder meta lock");
        let files: Vec<PathBuf> = files.into_iter().filter(|p| !inner.failed.contains(p)).collect();
        let same = |d: &Option<PathBuf>| d.as_ref() == Some(&dir);
        if files.is_empty() || same(&inner.running) || inner.job.as_ref().is_some_and(|(d, _)| *d == dir) {
            return;
        }
        inner.generation += 1;
        inner.job = Some((dir, files));
        if !inner.started {
            inner.started = true;
            let app = self.clone();
            std::thread::Builder::new()
                .name("folder-meta".into())
                .spawn(move || app.folder_meta_worker())
                .expect("spawn folder meta reader");
        }
        drop(inner);
        self.folder_meta.wake.notify_one();
    }

    /// A row from the early passes for a file not cached yet.
    pub(crate) fn early_folder_row(&self, canon: &Path) -> Option<TrackRow> {
        self.folder_meta.inner.lock().expect("folder meta lock").early.get(canon).map(FileMeta::to_row)
    }

    fn folder_meta_worker(self: Arc<Self>) {
        rille_engine::realtime::lower_current_thread();
        let mut lib = match self.open_library() {
            Ok(lib) => lib,
            Err(e) => {
                self.notify(UiEvent::Status(format!("Cannot read folder tags: {e}")));
                return;
            }
        };
        let cache = self.paths.cache.clone();
        // Serato databases read this session, by drive root.
        let mut serato: HashMap<PathBuf, SeratoDb> = HashMap::new();
        loop {
            let (dir, files, generation) = {
                let mut inner = self.folder_meta.inner.lock().expect("folder meta lock");
                inner.running = None;
                loop {
                    if self.is_shut_down() {
                        return;
                    }
                    if let Some((dir, files)) = inner.job.take() {
                        inner.running = Some(dir.clone());
                        break (dir, files, inner.generation);
                    }
                    inner = self.folder_meta.wake.wait(inner).expect("folder meta lock");
                }
            };
            let current = || self.folder_meta.inner.lock().expect("folder meta lock").generation == generation;
            let early = |metas: Vec<FileMeta>| {
                let mut inner = self.folder_meta.inner.lock().expect("folder meta lock");
                inner.early.extend(metas.into_iter().map(|m| (m.path.clone(), m)));
            };

            // Serato's data, in one read of its database.
            if let Some(root) = rille_library::serato_root(&dir) {
                let mtime = std::fs::metadata(root.join("_Serato_/database V2")).and_then(|m| m.modified()).ok();
                if serato.get(&root).is_none_or(|(t, _)| *t != mtime) {
                    let tracks = rille_library::serato_tracks(&root).unwrap_or_default();
                    serato.insert(root.clone(), (mtime, Arc::new(tracks)));
                }
                let tracks = &serato[&root].1;
                let known: Vec<FileMeta> = files.iter().filter_map(|p| tracks.get(p).cloned()).collect();
                if !known.is_empty() {
                    early(known);
                    self.notify(UiEvent::FolderMetaChanged(dir.clone()));
                }
            }

            // Tags without covers, for what Serato did not know.
            let unknown: Vec<PathBuf> = {
                let inner = self.folder_meta.inner.lock().expect("folder meta lock");
                files.iter().filter(|p| !inner.early.contains_key(*p)).cloned().collect()
            };
            self.read_folder_batches(&dir, &unknown, "Reading tags", &current, |chunk| {
                early(rille_library::read_metas_quick(chunk, QUICK_THREADS));
            });

            // Everything, covers included, into the cache. BPM and key from
            // the early passes stay where the tags have none.
            self.read_folder_batches(&dir, &files, "Reading covers", &current, |chunk| {
                let mut metas = rille_library::read_metas(chunk, &cache, THREADS);
                let mut inner = self.folder_meta.inner.lock().expect("folder meta lock");
                if metas.len() < chunk.len() {
                    let read: HashSet<&PathBuf> = metas.iter().map(|m| &m.path).collect();
                    let failed: Vec<PathBuf> = chunk.iter().filter(|p| !read.contains(p)).cloned().collect();
                    inner.failed.extend(failed);
                }
                for m in &mut metas {
                    if let Some(e) = inner.early.get(&m.path) {
                        m.tags.bpm = m.tags.bpm.or(e.tags.bpm);
                        m.tags.key = m.tags.key.or(e.tags.key);
                    }
                }
                drop(inner);
                if let Err(e) = lib.put_file_meta(&metas) {
                    self.notify(UiEvent::Status(format!("Cannot store folder tags: {e}")));
                    return;
                }
                // Cached now: the early data can go.
                let mut inner = self.folder_meta.inner.lock().expect("folder meta lock");
                for m in &metas {
                    inner.early.remove(&m.path);
                }
            });
        }
    }

    /// Reads `files` in batches with `read`, telling the browser as they come
    /// in, until done or a newer job arrives.
    fn read_folder_batches(
        &self,
        dir: &Path,
        files: &[PathBuf],
        what: &str,
        current: &dyn Fn() -> bool,
        mut read: impl FnMut(&[PathBuf]),
    ) {
        let total = files.len();
        let mut done = 0;
        let mut last_notify = Instant::now();
        let mut size = FIRST_BATCH;
        while done < total && current() && !self.is_shut_down() {
            let chunk = &files[done..(done + size).min(total)];
            read(chunk);
            done += chunk.len();
            size = BATCH;
            if done == chunk.len() || done == total || last_notify.elapsed() >= NOTIFY_EVERY {
                last_notify = Instant::now();
                self.notify(UiEvent::FolderMetaChanged(dir.to_owned()));
                if total > FIRST_BATCH {
                    let msg = if done < total {
                        format!("{what} {done}/{total} · {}", folder_name(dir))
                    } else {
                        format!("Read {total} files · {}", folder_name(dir))
                    };
                    self.notify(UiEvent::Status(msg));
                }
            }
        }
    }

    /// Wakes the folder reader so it can stop.
    pub(crate) fn stop_folder_meta(&self) {
        let _guard = self.folder_meta.inner.lock().expect("folder meta lock");
        self.folder_meta.wake.notify_all();
    }
}

fn folder_name(dir: &std::path::Path) -> String {
    dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned())
}
