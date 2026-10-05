//! Import folders and scanning.

use crate::library::{Stored, blob_path, path_blob, store_file};
use crate::meta::{self, FileInfo};
use crate::{Error, Library, Result, TrackId, io_err};
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Files read in parallel and committed in one transaction.
const BATCH: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanProgress {
    /// Walking the import folders.
    Walking { found: usize },
    /// Reading tags of new and changed files.
    Reading { done: usize, total: usize },
}

#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    /// Moved or renamed files matched to their existing track by content.
    pub relinked: usize,
    /// Tracks newly marked missing by this scan.
    pub missing: usize,
    pub unchanged: usize,
    /// Files that could not be read, or were imported with unreadable tags.
    pub errors: Vec<ScanError>,
}

/// Result of [`Library::import_files`].
#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    /// The collection id of every file that is now in the collection, in
    /// the order given (unsupported and unreadable files are left out).
    pub ids: Vec<TrackId>,
    pub added: usize,
    pub updated: usize,
    pub relinked: usize,
    pub unchanged: usize,
    pub errors: Vec<ScanError>,
}

#[derive(Clone, Debug)]
pub struct ScanError {
    pub path: PathBuf,
    pub message: String,
}

impl Library {
    pub fn add_root(&mut self, dir: &Path) -> Result<()> {
        let dir = dir.canonicalize().map_err(io_err(dir))?;
        if !dir.is_dir() {
            return Err(Error::Invalid("import folder is not a directory"));
        }
        self.conn.execute("INSERT OR IGNORE INTO roots (path) VALUES (?1)", [path_blob(&dir)])?;
        Ok(())
    }

    /// Stops scanning a folder. Its tracks stay in the collection.
    pub fn remove_root(&mut self, dir: &Path) -> Result<()> {
        self.conn.execute("DELETE FROM roots WHERE path = ?1", [path_blob(dir)])?;
        if let Ok(c) = dir.canonicalize() {
            self.conn.execute("DELETE FROM roots WHERE path = ?1", [path_blob(&c)])?;
        }
        Ok(())
    }

    pub fn roots(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT path FROM roots ORDER BY path")?;
        let roots = stmt.query_map([], |r| r.get(0).map(blob_path))?.collect::<rusqlite::Result<_>>()?;
        Ok(roots)
    }

    /// Walks the import folders: adds new files, refreshes changed ones
    /// (size or mtime), relinks moved files by content hash and marks tracks
    /// whose file is gone as missing. Unreadable files are reported, never
    /// fatal.
    pub fn scan(&mut self, progress: &mut dyn FnMut(ScanProgress)) -> Result<ScanReport> {
        let mut report = ScanReport::default();

        // 1. Walk.
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        for root in self.roots()? {
            let walk = WalkDir::new(&root).follow_links(true).into_iter();
            for entry in walk.filter_entry(|e| e.depth() == 0 || !e.file_name().as_encoded_bytes().starts_with(b".")) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        let path = e.path().unwrap_or(&root).to_owned();
                        report.errors.push(ScanError { path, message: e.to_string() });
                        continue;
                    }
                };
                if entry.file_type().is_file()
                    && meta::is_supported(entry.path())
                    && seen.insert(entry.path().to_owned())
                {
                    match meta::stat(entry.path()) {
                        Ok((size, mtime)) => found.push((entry.into_path(), size, mtime)),
                        Err(e) => report.errors.push(ScanError { path: entry.into_path(), message: e.to_string() }),
                    }
                    if found.len() % BATCH == 0 {
                        progress(ScanProgress::Walking { found: found.len() });
                    }
                }
            }
        }
        progress(ScanProgress::Walking { found: found.len() });

        // 2. Compare with the database.
        let mut known = HashMap::new();
        {
            // Guest tracks in an import folder join the collection.
            let mut stmt = self.conn.prepare("SELECT path, file_size, mtime, missing FROM tracks WHERE guest = 0")?;
            let rows = stmt.query_map([], |r| {
                Ok((blob_path(r.get(0)?), (r.get::<_, u64>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?)))
            })?;
            for row in rows {
                let (path, v) = row?;
                known.insert(path, v);
            }
        }
        let total = found.len();
        let work: Vec<PathBuf> = found
            .into_iter()
            .filter(|(p, size, mtime)| known.get(p) != Some(&(*size, *mtime, false)))
            .map(|(p, ..)| p)
            .collect();
        report.unchanged = total - work.len();

        // 3. Read new/changed files in parallel, store each batch in one transaction.
        let stored = self.store_paths(&work, progress)?;
        report.added = stored.added;
        report.updated = stored.updated;
        report.relinked = stored.relinked;
        report.errors.extend(stored.errors);

        // 4. Missing files: anything not seen in the walk whose file is gone.
        // Tracks outside the import folders (drag & drop) are checked too.
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare("SELECT id, path, missing FROM tracks")?;
            let rows: Vec<(TrackId, PathBuf, bool)> = stmt
                .query_map([], |r| Ok((r.get(0)?, blob_path(r.get(1)?), r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (id, path, missing) in rows {
                if seen.contains(&path) {
                    continue;
                }
                let gone = !path.exists();
                if gone != missing {
                    tx.execute("UPDATE tracks SET missing = ?2 WHERE id = ?1", params![id, gone])?;
                    report.missing += usize::from(gone);
                }
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// Adds files to the collection (the file explorer's import), refreshing
    /// changed ones and relinking moved ones like a scan does. Unsupported
    /// and unreadable files are reported, never fatal.
    pub fn import_files(&mut self, paths: &[PathBuf], progress: &mut dyn FnMut(ScanProgress)) -> Result<ImportReport> {
        let mut report = ImportReport::default();
        let mut wanted: Vec<PathBuf> = Vec::with_capacity(paths.len());
        for p in paths {
            if !meta::is_supported(p) {
                continue;
            }
            match p.canonicalize() {
                Ok(c) => wanted.push(c),
                Err(e) => report.errors.push(ScanError { path: p.clone(), message: e.to_string() }),
            }
        }
        // Unchanged known files need no reading.
        let mut work = Vec::new();
        for p in &wanted {
            let known = self
                .conn
                .query_row(
                    "SELECT file_size, mtime, missing FROM tracks WHERE path = ?1 AND guest = 0",
                    [path_blob(p)],
                    |r| Ok((r.get::<_, u64>(0)?, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?)),
                )
                .ok();
            match (known, meta::stat(p)) {
                (Some((size, mtime, false)), Ok(st)) if st == (size, mtime) => report.unchanged += 1,
                _ => work.push(p.clone()),
            }
        }
        let stored = self.store_paths(&work, progress)?;
        report.added = stored.added;
        report.updated = stored.updated;
        report.relinked = stored.relinked;
        report.errors.extend(stored.errors);
        for p in &wanted {
            if let Some(id) = self.track_by_path(p)? {
                report.ids.push(id);
            }
        }
        Ok(report)
    }

    /// Reads `work` in parallel and stores each batch in one transaction.
    fn store_paths(&mut self, work: &[PathBuf], progress: &mut dyn FnMut(ScanProgress)) -> Result<ImportReport> {
        let mut report = ImportReport::default();
        let mut done = 0;
        for chunk in work.chunks(BATCH) {
            progress(ScanProgress::Reading { done, total: work.len() });
            let infos = par_map(chunk, |p| read_guarded(p, &self.cache_dir));
            let tx = self.conn.transaction()?;
            for (path, info) in chunk.iter().zip(infos) {
                let info = match info {
                    Ok(info) => info,
                    Err(e) => {
                        report.errors.push(ScanError { path: path.clone(), message: e.to_string() });
                        continue;
                    }
                };
                if let Some(w) = &info.warning {
                    report.errors.push(ScanError { path: path.clone(), message: w.clone() });
                }
                match store_file(&tx, &info)?.1 {
                    Stored::Added => report.added += 1,
                    Stored::Updated => report.updated += 1,
                    Stored::Relinked => report.relinked += 1,
                }
            }
            tx.commit()?;
            done += chunk.len();
        }
        progress(ScanProgress::Reading { done, total: work.len() });
        Ok(report)
    }
}

/// Tag parsers can panic on malformed files; one bad file must not abort a scan.
fn read_guarded(path: &Path, cache_dir: &Path) -> Result<FileInfo, String> {
    match catch_unwind(AssertUnwindSafe(|| meta::read_file(path, cache_dir))) {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err("panic while reading file".into()),
    }
}

/// Order-preserving parallel map over scoped threads.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    par_map_n(items, std::thread::available_parallelism().map_or(1, |n| n.get()), f)
}

/// [`par_map`] over at most `threads` threads.
pub(crate) fn par_map_n<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = threads.max(1);
    let per = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = items.chunks(per).map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("scan worker panicked")).collect()
    })
}
