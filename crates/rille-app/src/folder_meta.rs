//! Tags and covers for explorer folders, read in the background so a folder
//! (a USB stick) shows its metadata without being imported. Results go to
//! the library's `file_meta` cache; the browser re-reads the folder when a
//! batch arrives.

use crate::{App, UiEvent};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Files read in parallel: USB sticks are slow at random reads.
const THREADS: usize = 4;
/// Files read before the cache is written; the first batch is small so the
/// visible rows fill in quickly.
const FIRST_BATCH: usize = 16;
const BATCH: usize = 64;
/// How often the browser is told to re-read the folder while reading.
const NOTIFY_EVERY: Duration = Duration::from_millis(400);

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
            let total = files.len();
            let mut done = 0;
            let mut last_notify = Instant::now();
            let mut size = FIRST_BATCH;
            while done < total && current() && !self.is_shut_down() {
                let chunk = &files[done..(done + size).min(total)];
                let metas = rille_library::read_metas(chunk, &cache, THREADS);
                if metas.len() < chunk.len() {
                    let read: HashSet<&PathBuf> = metas.iter().map(|m| &m.path).collect();
                    let mut inner = self.folder_meta.inner.lock().expect("folder meta lock");
                    inner.failed.extend(chunk.iter().filter(|p| !read.contains(p)).cloned());
                }
                if let Err(e) = lib.put_file_meta(&metas) {
                    self.notify(UiEvent::Status(format!("Cannot store folder tags: {e}")));
                }
                done += chunk.len();
                size = BATCH;
                if done == chunk.len() || done == total || last_notify.elapsed() >= NOTIFY_EVERY {
                    last_notify = Instant::now();
                    self.notify(UiEvent::FolderMetaChanged(dir.clone()));
                    if total > FIRST_BATCH {
                        let msg = if done < total {
                            format!("Reading tags {done}/{total} · {}", folder_name(&dir))
                        } else {
                            format!("Read tags of {total} files · {}", folder_name(&dir))
                        };
                        self.notify(UiEvent::Status(msg));
                    }
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
