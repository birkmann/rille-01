//! The [`Library`] handle: opening, track rows, single-file import and the
//! per-track data written by analysis and the user.

use crate::meta::{self, FileInfo};
use crate::{Error, Result, TrackId, io_err, now, schema};
use rille_core::track::ANALYZER_VERSION;
use rille_core::{BeatGrid, BeatMap, GridFlags, GridSource, Key, TrackAnalysis, TrackCues};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// The music collection. Not `Sync`; open one per thread if needed (WAL
/// allows concurrent readers).
pub struct Library {
    pub(crate) conn: Connection,
    pub(crate) cache_dir: PathBuf,
}

/// One row of the collection table.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackRow {
    pub id: TrackId,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub remixer: String,
    pub label: String,
    pub genre: String,
    pub comment: String,
    pub year: Option<i32>,
    pub duration_secs: f64,
    pub bpm: Option<f64>,
    pub key: Option<Key>,
    /// 0..=5 stars.
    pub rating: u8,
    /// 0xRRGGBB
    pub color: Option<u32>,
    pub play_count: u32,
    pub last_played: Option<i64>,
    pub date_added: i64,
    pub file_size: u64,
    /// kbit/s
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub has_cover: bool,
    /// Cover thumbnail key (see [`cover_thumb`]), so the UI finds the image
    /// without asking the database.
    pub cover: Option<String>,
    pub grid_confidence: Option<f32>,
    /// [`GridFlags`] bits.
    pub grid_flags: u32,
    pub grid_locked: bool,
    /// Has an analysis result (possibly from an older analyzer version).
    pub analyzed: bool,
    /// Analyzer version of that result.
    pub analysis_version: Option<u32>,
    /// The current analyzer failed on this file.
    pub analysis_failed: bool,
    pub missing: bool,
}

impl TrackRow {
    pub fn grid_flags(&self) -> GridFlags {
        GridFlags::from_bits_truncate(self.grid_flags)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverSize {
    /// 64 px, for table rows.
    Small,
    /// 256 px, for decks and the browser preview.
    Large,
}

impl CoverSize {
    pub fn px(self) -> u32 {
        match self {
            Self::Small => meta::COVER_SIZES[0],
            Self::Large => meta::COVER_SIZES[1],
        }
    }
}

/// `?V` is replaced by the analyzer version (see [`track_select`]).
const TRACK_SELECT: &str = "SELECT t.id, t.path, t.title, t.artist, t.album, t.remixer, t.label, t.genre, t.comment,
    t.year, t.duration, t.bpm, t.musical_key, t.rating, t.color, t.play_count, t.last_played, t.date_added,
    t.file_size, t.bitrate, t.sample_rate, t.cover, g.confidence, g.flags, g.locked,
    a.analyzer_version, e.track_id IS NOT NULL, t.missing
    FROM tracks t LEFT JOIN beatgrids g ON g.track_id = t.id LEFT JOIN analysis a ON a.track_id = t.id
    LEFT JOIN analysis_errors e ON e.track_id = t.id AND e.analyzer_version >= ?V";

fn track_select() -> String {
    TRACK_SELECT.replace("?V", &ANALYZER_VERSION.to_string())
}

/// The cached JPEG thumbnail for a [`TrackRow::cover`] key. The file may not
/// exist (cache cleared).
pub fn cover_thumb(cache_dir: &Path, cover: &str, size: CoverSize) -> PathBuf {
    meta::cover_file(cache_dir, cover, size.px())
}

fn track_row(r: &Row) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        id: r.get(0)?,
        path: blob_path(r.get(1)?),
        title: r.get(2)?,
        artist: r.get(3)?,
        album: r.get(4)?,
        remixer: r.get(5)?,
        label: r.get(6)?,
        genre: r.get(7)?,
        comment: r.get(8)?,
        year: r.get(9)?,
        duration_secs: r.get(10)?,
        bpm: r.get(11)?,
        key: r.get::<_, Option<u8>>(12)?.and_then(|k| Key::try_from(k).ok()),
        rating: r.get(13)?,
        color: r.get(14)?,
        play_count: r.get(15)?,
        last_played: r.get(16)?,
        date_added: r.get(17)?,
        file_size: r.get(18)?,
        bitrate: r.get(19)?,
        sample_rate: r.get(20)?,
        has_cover: r.get::<_, Option<String>>(21)?.is_some(),
        cover: r.get(21)?,
        grid_confidence: r.get(22)?,
        grid_flags: r.get::<_, Option<u32>>(23)?.unwrap_or(0),
        grid_locked: r.get::<_, Option<bool>>(24)?.unwrap_or(false),
        analyzed: r.get::<_, Option<u32>>(25)?.is_some(),
        analysis_version: r.get(25)?,
        analysis_failed: r.get(26)?,
        missing: r.get(27)?,
    })
}

pub(crate) fn path_blob(p: &Path) -> &[u8] {
    p.as_os_str().as_bytes()
}

pub(crate) fn blob_path(b: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(b))
}

impl Library {
    pub fn open(db_path: &Path, cache_dir: &Path) -> Result<Self> {
        if let Some(dir) = db_path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let conn = Connection::open(db_path)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn, cache_dir)
    }

    pub fn open_in_memory(cache_dir: &Path) -> Result<Self> {
        Self::init(Connection::open_in_memory()?, cache_dir)
    }

    fn init(mut conn: Connection, cache_dir: &Path) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > schema::VERSION {
            return Err(Error::Invalid("database was written by a newer version"));
        }
        schema::migrate(&mut conn)?;
        std::fs::create_dir_all(cache_dir).map_err(io_err(cache_dir))?;
        Ok(Self { conn, cache_dir: cache_dir.to_owned() })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// How long a write waits for another connection's transaction.
    pub fn set_busy_timeout(&self, t: std::time::Duration) -> Result<()> {
        Ok(self.conn.busy_timeout(t)?)
    }

    // -- Tracks ------------------------------------------------------------

    /// The whole collection, ordered by id.
    pub fn tracks(&self) -> Result<Vec<TrackRow>> {
        let mut stmt = self.conn.prepare(&format!("{} ORDER BY t.id", track_select()))?;
        let rows = stmt.query_map([], track_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn track(&self, id: TrackId) -> Result<Option<TrackRow>> {
        let sql = format!("{} WHERE t.id = ?1", track_select());
        Ok(self.conn.query_row(&sql, [id], track_row).optional()?)
    }

    /// Looks the path up as given, then canonicalized.
    pub fn track_by_path(&self, path: &Path) -> Result<Option<TrackId>> {
        let find = |p: &Path| {
            self.conn.query_row("SELECT id FROM tracks WHERE path = ?1", [path_blob(p)], |r| r.get(0)).optional()
        };
        match find(path)? {
            Some(id) => Ok(Some(id)),
            None => match path.canonicalize() {
                Ok(c) if c != path => Ok(find(&c)?),
                _ => Ok(None),
            },
        }
    }

    /// Adds a file (or refreshes it if its size/mtime changed) and returns
    /// its id. A file whose content matches a track whose file is gone is
    /// relinked to that track instead of being added twice.
    pub fn import_file(&mut self, path: &Path) -> Result<TrackId> {
        if !meta::is_supported(path) {
            return Err(Error::Unsupported(path.to_owned()));
        }
        let path = path.canonicalize().map_err(io_err(path))?;
        let known = self
            .conn
            .query_row("SELECT id, file_size, mtime, missing FROM tracks WHERE path = ?1", [path_blob(&path)], |r| {
                Ok((r.get::<_, TrackId>(0)?, r.get::<_, u64>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?))
            })
            .optional()?;
        if let Some((id, size, mtime, false)) = known {
            if meta::stat(&path).map_err(io_err(&path))? == (size, mtime) {
                return Ok(id);
            }
        }
        let info = meta::read_file(&path, &self.cache_dir)?;
        let tx = self.conn.transaction()?;
        let (id, _) = store_file(&tx, &info)?;
        tx.commit()?;
        Ok(id)
    }

    /// Removes tracks from the collection (not from disk), including their
    /// playlist and history entries.
    pub fn remove_tracks(&mut self, ids: &[TrackId]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM tracks WHERE id = ?1", [id])?;
        }
        Ok(tx.commit()?)
    }

    /// The cached JPEG thumbnail, if the track has a cover.
    pub fn cover_path(&self, id: TrackId, size: CoverSize) -> Option<PathBuf> {
        let cover: String =
            self.conn.query_row("SELECT cover FROM tracks WHERE id = ?1", [id], |r| r.get(0)).ok().flatten()?;
        Some(meta::cover_file(&self.cache_dir, &cover, size.px())).filter(|p| p.exists())
    }

    // -- Analysis and grids ------------------------------------------------

    /// Present tracks without an analysis from the current analyzer (and
    /// that the current analyzer has not failed on).
    pub fn tracks_needing_analysis(&self) -> Result<Vec<TrackId>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tracks t LEFT JOIN analysis a ON a.track_id = t.id
             LEFT JOIN analysis_errors e ON e.track_id = t.id AND e.analyzer_version >= ?1
             WHERE t.missing = 0 AND (a.analyzer_version IS NULL OR a.analyzer_version < ?1)
             AND e.track_id IS NULL ORDER BY t.id",
        )?;
        let ids = stmt.query_map([ANALYZER_VERSION], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Stores an analysis result and updates the sortable columns. The
    /// analyzer's grid becomes the track's grid unless the current one is
    /// protected: edited by the user, locked, or imported from Traktor.
    pub fn set_analysis(&mut self, id: TrackId, a: &TrackAnalysis) -> Result<()> {
        let tx = self.conn.transaction()?;
        ensure_track(&tx, id)?;
        tx.execute(
            "INSERT OR REPLACE INTO analysis (track_id, analyzer_version, data) VALUES (?1, ?2, ?3)",
            params![id, a.analyzer_version, serde_json::to_string(a)?],
        )?;
        tx.execute(
            "UPDATE tracks SET musical_key = COALESCE(?2, musical_key),
             duration = CASE WHEN ?3 > 0 THEN ?3 ELSE duration END WHERE id = ?1",
            params![id, a.key.map(u8::from), a.duration_secs],
        )?;
        if !grid_protected(&tx, id)? {
            match &a.grid {
                Some(g) => write_grid(&tx, id, g, false)?,
                None => clear_grid(&tx, id)?,
            }
        }
        tx.execute("DELETE FROM analysis_errors WHERE track_id = ?1", [id])?;
        Ok(tx.commit()?)
    }

    /// Records that the current analyzer failed on a track, so it is not
    /// retried until the analyzer changes (or the user asks).
    pub fn set_analysis_error(&mut self, id: TrackId, message: &str) -> Result<()> {
        ensure_track(&self.conn, id)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO analysis_errors (track_id, analyzer_version, message, at) VALUES (?1, ?2, ?3, ?4)",
            params![id, ANALYZER_VERSION, message, now()],
        )?;
        Ok(())
    }

    /// Why the current analyzer failed on a track, if it did.
    pub fn analysis_error(&self, id: TrackId) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT message FROM analysis_errors WHERE track_id = ?1 AND analyzer_version >= ?2",
                params![id, ANALYZER_VERSION],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Tracks whose file lies in `dir` (directly, or anywhere below it with
    /// `recursive`), ordered by path.
    pub fn tracks_under(&self, dir: &Path, recursive: bool) -> Result<Vec<TrackRow>> {
        let mut prefix = path_blob(dir).to_vec();
        if prefix.last() != Some(&b'/') {
            prefix.push(b'/');
        }
        // All paths starting with `prefix`: prefix <= path < prefix with its
        // last byte ('/') raised by one.
        let mut upper = prefix.clone();
        *upper.last_mut().expect("non-empty") += 1;
        let sql = format!("{} WHERE t.path >= ?1 AND t.path < ?2 ORDER BY t.path", track_select());
        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<TrackRow> =
            stmt.query_map(params![prefix, upper], track_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows.into_iter().filter(|r| recursive || !path_blob(&r.path)[prefix.len()..].contains(&b'/')).collect())
    }

    /// The stored analysis, with `grid` replaced by the track's effective
    /// grid (which may be user-edited or imported).
    pub fn analysis(&self, id: TrackId) -> Result<Option<TrackAnalysis>> {
        let data: Option<String> =
            self.conn.query_row("SELECT data FROM analysis WHERE track_id = ?1", [id], |r| r.get(0)).optional()?;
        let Some(data) = data else { return Ok(None) };
        let mut a: TrackAnalysis = serde_json::from_str(&data)?;
        a.grid = self.grid(id)?;
        Ok(Some(a))
    }

    /// The effective grid.
    pub fn grid(&self, id: TrackId) -> Result<Option<BeatGrid>> {
        let json: Option<String> =
            self.conn.query_row("SELECT grid FROM beatgrids WHERE track_id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    /// Stores a grid edited by the user; re-analysis will not replace it.
    pub fn set_grid(&mut self, id: TrackId, grid: &BeatGrid) -> Result<()> {
        let tx = self.conn.transaction()?;
        ensure_track(&tx, id)?;
        write_grid(&tx, id, grid, true)?;
        Ok(tx.commit()?)
    }

    /// Drops the user/imported grid and falls back to the analyzer's grid.
    pub fn reset_grid(&mut self, id: TrackId) -> Result<()> {
        let auto =
            self.conn.query_row("SELECT data FROM analysis WHERE track_id = ?1", [id], |r| r.get::<_, String>(0));
        let auto =
            auto.optional()?.map(|d| serde_json::from_str::<TrackAnalysis>(&d)).transpose()?.and_then(|a| a.grid);
        let tx = self.conn.transaction()?;
        match auto {
            Some(g) => write_grid(&tx, id, &g, false)?,
            None => clear_grid(&tx, id)?,
        }
        Ok(tx.commit()?)
    }

    /// Opaque waveform/overview blob produced by the analyzer.
    pub fn set_waveform(&mut self, id: TrackId, data: &[u8]) -> Result<()> {
        ensure_track(&self.conn, id)?;
        self.conn.execute("INSERT OR REPLACE INTO waveforms (track_id, data) VALUES (?1, ?2)", params![id, data])?;
        Ok(())
    }

    pub fn waveform(&self, id: TrackId) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row("SELECT data FROM waveforms WHERE track_id = ?1", [id], |r| r.get(0)).optional()?)
    }

    // -- User data ---------------------------------------------------------

    pub fn cues(&self, id: TrackId) -> Result<TrackCues> {
        let json: Option<String> = self
            .conn
            .query_row("SELECT cues FROM tracks WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or(Error::NoTrack(id))?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?.unwrap_or_default())
    }

    pub fn set_cues(&mut self, id: TrackId, cues: &TrackCues) -> Result<()> {
        update_track(&self.conn, id, "cues", serde_json::to_string(cues)?)
    }

    /// 0..=5 stars (clamped).
    pub fn set_rating(&mut self, id: TrackId, stars: u8) -> Result<()> {
        update_track(&self.conn, id, "rating", stars.min(5))
    }

    pub fn set_color(&mut self, id: TrackId, color: Option<u32>) -> Result<()> {
        update_track(&self.conn, id, "color", color)
    }
}

// ---------------------------------------------------------------------------
// Helpers shared with scan and NML import
// ---------------------------------------------------------------------------

pub(crate) fn ensure_track(conn: &Connection, id: TrackId) -> Result<()> {
    let found = conn.query_row("SELECT 1 FROM tracks WHERE id = ?1", [id], |_| Ok(())).optional()?;
    found.ok_or(Error::NoTrack(id))
}

fn update_track(conn: &Connection, id: TrackId, column: &str, value: impl rusqlite::ToSql) -> Result<()> {
    let n = conn.execute(&format!("UPDATE tracks SET {column} = ?2 WHERE id = ?1"), params![id, value])?;
    if n == 0 { Err(Error::NoTrack(id)) } else { Ok(()) }
}

/// Whether re-analysis must leave the stored grid alone.
pub(crate) fn grid_protected(conn: &Connection, id: TrackId) -> Result<bool> {
    let row = conn
        .query_row("SELECT user_edited, locked, source FROM beatgrids WHERE track_id = ?1", [id], |r| {
            Ok(r.get::<_, bool>(0)? || r.get::<_, bool>(1)? || r.get::<_, String>(2)? == "imported_nml")
        })
        .optional()?;
    Ok(row.unwrap_or(false))
}

fn clear_grid(conn: &Connection, id: TrackId) -> Result<()> {
    conn.execute("DELETE FROM beatgrids WHERE track_id = ?1", [id])?;
    conn.execute("UPDATE tracks SET bpm = NULL WHERE id = ?1", [id])?;
    Ok(())
}

/// Writes the effective grid and the derived bpm column.
pub(crate) fn write_grid(conn: &Connection, id: TrackId, g: &BeatGrid, user_edited: bool) -> Result<()> {
    let source = match g.source {
        GridSource::Auto { .. } => "auto",
        GridSource::Manual => "manual",
        GridSource::ImportedNml => "imported_nml",
        GridSource::Tapped => "tapped",
    };
    conn.execute(
        "INSERT OR REPLACE INTO beatgrids (track_id, grid, user_edited, locked, source, confidence, flags)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, serde_json::to_string(g)?, user_edited, g.locked, source, g.confidence, g.flags.bits()],
    )?;
    let duration: f64 = conn.query_row("SELECT duration FROM tracks WHERE id = ?1", [id], |r| r.get(0))?;
    conn.execute("UPDATE tracks SET bpm = ?2 WHERE id = ?1", params![id, grid_bpm(g, duration)])?;
    Ok(())
}

/// The tempo shown in the library: for variable grids, the tempo that covers
/// most of the track.
pub(crate) fn grid_bpm(g: &BeatGrid, duration: f64) -> f64 {
    match &g.map {
        BeatMap::Constant(m) => m.bpm(),
        BeatMap::Piecewise(m) => {
            let segs = m.segments();
            let span = |i: usize| {
                let end = segs.get(i + 1).map_or(duration.max(segs[i].start_secs), |s| s.start_secs);
                end - if i == 0 { 0.0f64.min(segs[0].start_secs) } else { segs[i].start_secs }
            };
            let best = (0..segs.len()).fold(0, |best, i| if span(i) > span(best) { i } else { best });
            segs[best].bpm
        }
        BeatMap::Live(m) => {
            let b = m.beats_secs();
            60.0 * (b.len() - 1) as f64 / (b[b.len() - 1] - b[0])
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stored {
    Added,
    Updated,
    Relinked,
}

/// Inserts or refreshes a file's row. Tag-derived bpm/key only fill empty
/// columns so analysis results win.
pub(crate) fn store_file(conn: &Connection, info: &FileInfo) -> Result<(TrackId, Stored)> {
    let blob = path_blob(&info.path);
    let existing = conn.query_row("SELECT id FROM tracks WHERE path = ?1", [blob], |r| r.get(0)).optional()?;
    let (id, how) = match existing {
        Some(id) => (id, Stored::Updated),
        None => {
            // A track with the same content whose file is gone was moved here.
            let mut stmt =
                conn.prepare_cached("SELECT id, path FROM tracks WHERE content_hash = ?1 AND file_size = ?2")?;
            let candidates: Vec<(TrackId, Vec<u8>)> = stmt
                .query_map(params![info.hash as i64, info.size], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            match candidates.into_iter().find(|(_, p)| !blob_path(p.clone()).exists()) {
                Some((id, _)) => (id, Stored::Relinked),
                None => {
                    conn.execute(
                        "INSERT INTO tracks (path, file_size, mtime, content_hash, date_added) VALUES (?1, 0, 0, 0, ?2)",
                        params![blob, now()],
                    )?;
                    (conn.last_insert_rowid(), Stored::Added)
                }
            }
        }
    };
    let t = &info.tags;
    conn.execute(
        "UPDATE tracks SET path = ?2, file_size = ?3, mtime = ?4, content_hash = ?5, title = ?6, artist = ?7,
         album = ?8, remixer = ?9, label = ?10, genre = ?11, comment = ?12, year = ?13, duration = ?14,
         bitrate = ?15, sample_rate = ?16, bpm = COALESCE(bpm, ?17), musical_key = COALESCE(musical_key, ?18),
         cover = ?19, missing = 0 WHERE id = ?1",
        params![
            id,
            blob,
            info.size,
            info.mtime,
            info.hash as i64,
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
            info.cover,
        ],
    )?;
    Ok((id, how))
}
