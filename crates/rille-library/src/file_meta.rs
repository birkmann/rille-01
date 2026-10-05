//! Tags and covers of browsed files outside the collection, cached so a
//! folder (a USB stick) shows its metadata without being imported.

use crate::library::{blob_path, path_blob};
use crate::meta::{self, FileMeta, Tags};
use crate::scan::par_map_n;
use crate::{Library, Result, TrackRow, now};
use rille_core::Key;
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

impl Library {
    /// Cached metadata for `paths`, keyed by the path as given. Entries whose
    /// file changed size or mtime since (or is gone) are left out.
    pub fn file_meta(&self, paths: &[PathBuf]) -> Result<HashMap<PathBuf, FileMeta>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT file_size, mtime, title, artist, album, remixer, label, genre, comment, year, duration,
             bitrate, sample_rate, bpm, musical_key, cover FROM file_meta WHERE path = ?1",
        )?;
        let mut out = HashMap::new();
        for p in paths {
            let Ok(st) = meta::stat(p) else { continue };
            let row = stmt
                .query_row([path_blob(p)], |r| {
                    Ok(FileMeta {
                        path: p.clone(),
                        size: r.get(0)?,
                        mtime: r.get(1)?,
                        tags: Tags {
                            title: r.get(2)?,
                            artist: r.get(3)?,
                            album: r.get(4)?,
                            remixer: r.get(5)?,
                            label: r.get(6)?,
                            genre: r.get(7)?,
                            comment: r.get(8)?,
                            year: r.get(9)?,
                            duration_secs: r.get(10)?,
                            bitrate: r.get(11)?,
                            sample_rate: r.get(12)?,
                            bpm: r.get(13)?,
                            key: r.get::<_, Option<u8>>(14)?.and_then(|k| Key::try_from(k).ok()),
                        },
                        cover: r.get(15)?,
                    })
                })
                .optional()?;
            if let Some(m) = row.filter(|m| (m.size, m.mtime) == st) {
                out.insert(p.clone(), m);
            }
        }
        Ok(out)
    }

    /// Stores read metadata, replacing older entries for the same paths.
    pub fn put_file_meta(&mut self, metas: &[FileMeta]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO file_meta (path, file_size, mtime, title, artist, album, remixer, label,
                 genre, comment, year, duration, bitrate, sample_rate, bpm, musical_key, cover, read_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            )?;
            for m in metas {
                let t = &m.tags;
                stmt.execute(params![
                    path_blob(&m.path),
                    m.size,
                    m.mtime,
                    t.title,
                    t.artist,
                    t.album,
                    t.remixer,
                    t.label,
                    t.genre,
                    t.comment,
                    t.year,
                    t.duration_secs,
                    t.bitrate,
                    t.sample_rate,
                    t.bpm,
                    t.key.map(u8::from),
                    m.cover,
                    now(),
                ])?;
            }
        }
        Ok(tx.commit()?)
    }

    /// Forgets cached metadata of files below `dir`.
    pub fn clear_file_meta_under(&mut self, dir: &Path) -> Result<()> {
        let mut prefix = path_blob(dir).to_vec();
        if prefix.last() != Some(&b'/') {
            prefix.push(b'/');
        }
        let mut upper = prefix.clone();
        *upper.last_mut().expect("non-empty") += 1;
        self.conn.execute("DELETE FROM file_meta WHERE path >= ?1 AND path < ?2", params![prefix, upper])?;
        Ok(())
    }

    /// Paths of all cached entries (for tests and maintenance).
    pub fn file_meta_paths(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT path FROM file_meta ORDER BY path")?;
        let paths = stmt.query_map([], |r| r.get(0).map(blob_path))?.collect::<rusqlite::Result<_>>()?;
        Ok(paths)
    }
}

/// Reads the metadata of `paths` on up to `threads` threads, in order.
/// Unreadable files are left out; a tag parser panic is caught.
pub fn read_metas(paths: &[PathBuf], cache_dir: &Path, threads: usize) -> Vec<FileMeta> {
    par_map_n(paths, threads, |p| catch_unwind(AssertUnwindSafe(|| meta::read_meta(p, cache_dir))).ok()?.ok())
        .into_iter()
        .flatten()
        .collect()
}

impl FileMeta {
    /// A browser row for a file outside the collection (id −1).
    pub fn to_row(&self) -> TrackRow {
        let t = &self.tags;
        TrackRow {
            id: -1,
            path: self.path.clone(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            remixer: t.remixer.clone(),
            label: t.label.clone(),
            genre: t.genre.clone(),
            comment: t.comment.clone(),
            year: t.year,
            duration_secs: t.duration_secs,
            bpm: t.bpm,
            key: t.key,
            rating: 0,
            color: None,
            play_count: 0,
            last_played: None,
            date_added: 0,
            file_size: self.size,
            bitrate: t.bitrate,
            sample_rate: t.sample_rate,
            has_cover: self.cover.is_some(),
            cover: self.cover.clone(),
            grid_confidence: None,
            grid_flags: 0,
            grid_locked: false,
            analyzed: false,
            analysis_version: None,
            analysis_failed: false,
            missing: false,
            beatport_id: None,
            beatport_offline: false,
            guest: false,
        }
    }
}
