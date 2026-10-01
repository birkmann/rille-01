//! Schema and migrations, versioned with `PRAGMA user_version`.

use rusqlite::Connection;

/// Migration `i` upgrades a database from version `i` to `i + 1`. Only ever
/// append to this list.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE roots (path BLOB PRIMARY KEY) WITHOUT ROWID;

CREATE TABLE tracks (
    id INTEGER PRIMARY KEY,
    path BLOB NOT NULL UNIQUE,
    file_size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    -- xxh3 of size + sampled content (bit-cast to i64), for relinking moved files
    content_hash INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    artist TEXT NOT NULL DEFAULT '',
    album TEXT NOT NULL DEFAULT '',
    remixer TEXT NOT NULL DEFAULT '',
    label TEXT NOT NULL DEFAULT '',
    genre TEXT NOT NULL DEFAULT '',
    comment TEXT NOT NULL DEFAULT '',
    year INTEGER,
    duration REAL NOT NULL DEFAULT 0,
    bitrate INTEGER,
    sample_rate INTEGER,
    bpm REAL,
    musical_key INTEGER,
    rating INTEGER NOT NULL DEFAULT 0,
    color INTEGER,
    play_count INTEGER NOT NULL DEFAULT 0,
    last_played INTEGER,
    date_added INTEGER NOT NULL,
    cover TEXT,
    missing INTEGER NOT NULL DEFAULT 0,
    cues TEXT
);
CREATE INDEX tracks_content_hash ON tracks(content_hash);

-- Latest analysis result as JSON, including the analyzer's own grid.
CREATE TABLE analysis (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    analyzer_version INTEGER NOT NULL,
    data TEXT NOT NULL
);

-- The effective grid. Re-analysis must not replace a protected one
-- (user_edited, locked or imported from Traktor).
CREATE TABLE beatgrids (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    grid TEXT NOT NULL,
    user_edited INTEGER NOT NULL,
    locked INTEGER NOT NULL,
    source TEXT NOT NULL,
    confidence REAL NOT NULL,
    flags INTEGER NOT NULL
);

CREATE TABLE waveforms (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    data BLOB NOT NULL
);

CREATE TABLE playlists (
    id INTEGER PRIMARY KEY,
    parent INTEGER REFERENCES playlists(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    is_folder INTEGER NOT NULL,
    sort INTEGER NOT NULL
);

CREATE TABLE playlist_entries (
    playlist INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    pos INTEGER NOT NULL,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    PRIMARY KEY (playlist, pos)
) WITHOUT ROWID;
CREATE INDEX playlist_entries_track ON playlist_entries(track_id);

CREATE TABLE history_sessions (
    id INTEGER PRIMARY KEY,
    started INTEGER NOT NULL
);

CREATE TABLE history_entries (
    id INTEGER PRIMARY KEY,
    session INTEGER NOT NULL REFERENCES history_sessions(id) ON DELETE CASCADE,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    deck INTEGER NOT NULL,
    played_at INTEGER NOT NULL
);
CREATE INDEX history_entries_session ON history_entries(session);
"#,
    r#"
-- Files the analyzer could not handle; retried when the analyzer changes.
CREATE TABLE analysis_errors (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    analyzer_version INTEGER NOT NULL,
    message TEXT NOT NULL,
    at INTEGER NOT NULL
);
"#,
];

pub const VERSION: u32 = MIGRATIONS.len() as u32;

pub fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as u32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
