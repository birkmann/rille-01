//! Music collection database: import folders and scanning, tag reading and
//! cover thumbnails, analysis/beatgrid/cue storage, playlists, play history
//! and Traktor NML import.
//!
//! Everything lives in one SQLite database (WAL mode). Paths are stored as
//! raw bytes so non-UTF-8 file names survive; timestamps are unix seconds.
//! Structured data (analysis, grids, cues) is stored as JSON next to plain
//! columns that the library table sorts by.

mod library;
mod meta;
mod nml;
mod playlists;
mod scan;
mod schema;
mod search;

pub use library::{CoverSize, Library, TrackRow, cover_thumb};
pub use meta::{SUPPORTED_EXTENSIONS, is_supported, parse_key};
pub use nml::NmlReport;
pub use playlists::{HistoryEntry, HistorySession, PlaylistNode};
pub use scan::{ImportReport, ScanError, ScanProgress, ScanReport};
pub use search::SearchIndex;

use std::path::PathBuf;

pub type TrackId = i64;
pub type PlaylistId = i64;
pub type SessionId = i64;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("nml: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("unsupported file type: {}", .0.display())]
    Unsupported(PathBuf),
    #[error("no track with id {0}")]
    NoTrack(TrackId),
    #[error("no playlist with id {0}")]
    NoPlaylist(PlaylistId),
    #[error("{0}")]
    Invalid(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn io_err(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}
