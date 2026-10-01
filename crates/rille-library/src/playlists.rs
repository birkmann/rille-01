//! Playlist tree (folders and playlists) and play history.

use crate::library::ensure_track;
use crate::{Error, Library, PlaylistId, Result, SessionId, TrackId, now};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone, Debug, PartialEq)]
pub struct PlaylistNode {
    pub id: PlaylistId,
    pub parent: Option<PlaylistId>,
    pub name: String,
    pub is_folder: bool,
    /// Entries of a playlist; 0 for folders.
    pub track_count: usize,
    pub children: Vec<PlaylistNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistorySession {
    pub id: SessionId,
    /// Unix seconds.
    pub started: i64,
    pub track_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub track: TrackId,
    pub deck: u8,
    pub played_at: i64,
}

impl Library {
    // -- Playlists ---------------------------------------------------------

    /// Top-level nodes in display order, children nested.
    pub fn playlist_tree(&self) -> Result<Vec<PlaylistNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.parent, p.name, p.is_folder, (SELECT COUNT(*) FROM playlist_entries e WHERE e.playlist = p.id)
             FROM playlists p ORDER BY p.sort, p.id",
        )?;
        let flat: Vec<PlaylistNode> = stmt
            .query_map([], |r| {
                Ok(PlaylistNode {
                    id: r.get(0)?,
                    parent: r.get(1)?,
                    name: r.get(2)?,
                    is_folder: r.get(3)?,
                    track_count: r.get(4)?,
                    children: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        fn build(flat: &[PlaylistNode], parent: Option<PlaylistId>) -> Vec<PlaylistNode> {
            let kids = flat.iter().filter(|n| n.parent == parent);
            kids.map(|n| PlaylistNode { children: build(flat, Some(n.id)), ..n.clone() }).collect()
        }
        Ok(build(&flat, None))
    }

    /// Appends a playlist or folder under `parent` (a folder, or the top level).
    pub fn create_playlist(&mut self, parent: Option<PlaylistId>, name: &str, is_folder: bool) -> Result<PlaylistId> {
        if let Some(p) = parent {
            require_folder(&self.conn, p)?;
        }
        self.conn.execute(
            "INSERT INTO playlists (parent, name, is_folder, sort)
             VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(sort), -1) + 1 FROM playlists WHERE parent IS ?1))",
            params![parent, name, is_folder],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The first node named `name` of this kind under `parent`, created
    /// when there is none (re-importing a folder reuses its playlist).
    pub fn playlist_named(&mut self, parent: Option<PlaylistId>, name: &str, is_folder: bool) -> Result<PlaylistId> {
        let found = self
            .conn
            .query_row(
                "SELECT id FROM playlists WHERE parent IS ?1 AND name = ?2 AND is_folder = ?3 ORDER BY sort, id",
                params![parent, name, is_folder],
                |r| r.get(0),
            )
            .optional()?;
        match found {
            Some(id) => Ok(id),
            None => self.create_playlist(parent, name, is_folder),
        }
    }

    /// Appends the `tracks` the playlist does not hold yet, in order.
    /// Returns how many were added.
    pub fn add_missing_to_playlist(&mut self, playlist: PlaylistId, tracks: &[TrackId]) -> Result<usize> {
        let mut added = 0;
        self.edit_playlist(playlist, |list| {
            for &t in tracks {
                if !list.contains(&t) {
                    list.push(t);
                    added += 1;
                }
            }
            Ok(())
        })?;
        Ok(added)
    }

    pub fn rename_playlist(&mut self, id: PlaylistId, name: &str) -> Result<()> {
        match self.conn.execute("UPDATE playlists SET name = ?2 WHERE id = ?1", params![id, name])? {
            0 => Err(Error::NoPlaylist(id)),
            _ => Ok(()),
        }
    }

    /// Deletes a playlist, or a folder with everything in it.
    pub fn delete_playlist(&mut self, id: PlaylistId) -> Result<()> {
        match self.conn.execute("DELETE FROM playlists WHERE id = ?1", [id])? {
            0 => Err(Error::NoPlaylist(id)),
            _ => Ok(()),
        }
    }

    /// Moves a node to the end of another folder (or the top level).
    pub fn move_playlist(&mut self, id: PlaylistId, parent: Option<PlaylistId>) -> Result<()> {
        node_kind(&self.conn, id)?;
        // Walk up from the new parent to refuse moving a folder into itself.
        let mut up = parent;
        while let Some(p) = up {
            if p == id {
                return Err(Error::Invalid("cannot move a folder into itself"));
            }
            require_folder(&self.conn, p)?;
            up = self.conn.query_row("SELECT parent FROM playlists WHERE id = ?1", [p], |r| r.get(0))?;
        }
        self.conn.execute(
            "UPDATE playlists SET parent = ?2,
             sort = (SELECT COALESCE(MAX(sort), -1) + 1 FROM playlists WHERE parent IS ?2) WHERE id = ?1",
            params![id, parent],
        )?;
        Ok(())
    }

    pub fn playlist_tracks(&self, playlist: PlaylistId) -> Result<Vec<TrackId>> {
        node_kind(&self.conn, playlist)?;
        entries(&self.conn, playlist)
    }

    /// Inserts `tracks` at position `at` (appends when `None` or past the end).
    /// Duplicates are allowed.
    pub fn add_to_playlist(&mut self, playlist: PlaylistId, tracks: &[TrackId], at: Option<usize>) -> Result<()> {
        for t in tracks {
            ensure_track(&self.conn, *t)?;
        }
        self.edit_playlist(playlist, |list| {
            let at = at.unwrap_or(list.len()).min(list.len());
            list.splice(at..at, tracks.iter().copied());
            Ok(())
        })
    }

    /// Removes the entries at `positions`; out-of-range positions are ignored.
    pub fn remove_from_playlist(&mut self, playlist: PlaylistId, positions: &[usize]) -> Result<()> {
        self.edit_playlist(playlist, |list| {
            let mut i = 0;
            list.retain(|_| {
                i += 1;
                !positions.contains(&(i - 1))
            });
            Ok(())
        })
    }

    /// Moves the entry at `from` so that it ends up at index `to` (clamped).
    pub fn move_in_playlist(&mut self, playlist: PlaylistId, from: usize, to: usize) -> Result<()> {
        self.edit_playlist(playlist, |list| {
            if from >= list.len() {
                return Err(Error::Invalid("playlist position out of range"));
            }
            let t = list.remove(from);
            list.insert(to.min(list.len()), t);
            Ok(())
        })
    }

    /// Load, edit, rewrite: playlists are small, and this keeps positions dense.
    fn edit_playlist(
        &mut self,
        playlist: PlaylistId,
        edit: impl FnOnce(&mut Vec<TrackId>) -> Result<()>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        if node_kind(&tx, playlist)? {
            return Err(Error::Invalid("folders have no tracks"));
        }
        let mut list = entries(&tx, playlist)?;
        edit(&mut list)?;
        tx.execute("DELETE FROM playlist_entries WHERE playlist = ?1", [playlist])?;
        {
            let mut ins = tx.prepare("INSERT INTO playlist_entries (playlist, pos, track_id) VALUES (?1, ?2, ?3)")?;
            for (pos, t) in list.iter().enumerate() {
                ins.execute(params![playlist, pos, t])?;
            }
        }
        Ok(tx.commit()?)
    }

    // -- History -----------------------------------------------------------

    pub fn start_history_session(&mut self) -> Result<SessionId> {
        self.conn.execute("INSERT INTO history_sessions (started) VALUES (?1)", [now()])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Records a play and bumps the track's play count and last-played time.
    pub fn log_played(&mut self, session: SessionId, id: TrackId, deck: u8) -> Result<()> {
        let tx = self.conn.transaction()?;
        let t = now();
        tx.execute(
            "INSERT INTO history_entries (session, track_id, deck, played_at) VALUES (?1, ?2, ?3, ?4)",
            params![session, id, deck, t],
        )
        .map_err(|e| match e.sqlite_error_code() {
            Some(rusqlite::ErrorCode::ConstraintViolation) => Error::Invalid("unknown history session or track"),
            _ => e.into(),
        })?;
        tx.execute("UPDATE tracks SET play_count = play_count + 1, last_played = ?2 WHERE id = ?1", params![id, t])?;
        Ok(tx.commit()?)
    }

    /// Newest first.
    pub fn history_sessions(&self) -> Result<Vec<HistorySession>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.started, (SELECT COUNT(*) FROM history_entries e WHERE e.session = s.id)
             FROM history_sessions s ORDER BY s.started DESC, s.id DESC",
        )?;
        let rows = stmt
            .query_map([], |r| Ok(HistorySession { id: r.get(0)?, started: r.get(1)?, track_count: r.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Tracks in play order.
    pub fn history_tracks(&self, session: SessionId) -> Result<Vec<TrackId>> {
        Ok(self.history_entries(session)?.into_iter().map(|e| e.track).collect())
    }

    pub fn history_entries(&self, session: SessionId) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT track_id, deck, played_at FROM history_entries WHERE session = ?1 ORDER BY id")?;
        let rows = stmt
            .query_map([session], |r| Ok(HistoryEntry { track: r.get(0)?, deck: r.get(1)?, played_at: r.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn delete_history_session(&mut self, session: SessionId) -> Result<()> {
        self.conn.execute("DELETE FROM history_sessions WHERE id = ?1", [session])?;
        Ok(())
    }
}

/// `is_folder` of a node, or `NoPlaylist`.
fn node_kind(conn: &Connection, id: PlaylistId) -> Result<bool> {
    let kind = conn.query_row("SELECT is_folder FROM playlists WHERE id = ?1", [id], |r| r.get(0)).optional()?;
    kind.ok_or(Error::NoPlaylist(id))
}

fn require_folder(conn: &Connection, id: PlaylistId) -> Result<()> {
    if node_kind(conn, id)? { Ok(()) } else { Err(Error::Invalid("parent is not a folder")) }
}

fn entries(conn: &Connection, playlist: PlaylistId) -> Result<Vec<TrackId>> {
    let mut stmt = conn.prepare_cached("SELECT track_id FROM playlist_entries WHERE playlist = ?1 ORDER BY pos")?;
    let ids = stmt.query_map([playlist], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}
