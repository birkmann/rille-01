//! Reads BPM, key and constant beatgrids from a Mixxx library database.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct MixxxTrack {
    pub bpm: f64,
    /// First beat in seconds (BeatGrid-2.0 only).
    pub first_beat_secs: Option<f64>,
}

pub fn load(db: &Path) -> rusqlite::Result<HashMap<PathBuf, MixxxTrack>> {
    let conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT t.location, l.bpm, COALESCE(l.key, ''), l.beats, COALESCE(l.beats_version, ''), l.samplerate
         FROM library l JOIN track_locations t ON l.location = t.id WHERE l.bpm > 0",
    )?;
    let rows = stmt.query_map([], |r| {
        let path: String = r.get(0)?;
        let blob: Option<Vec<u8>> = r.get(3)?;
        let version: String = r.get(4)?;
        let sr: f64 = r.get::<_, Option<f64>>(5)?.unwrap_or(44_100.0);
        let first_beat_secs =
            if version == "BeatGrid-2.0" { blob.as_deref().and_then(first_beat_frame).map(|f| f / sr) } else { None };
        Ok((PathBuf::from(path), MixxxTrack { bpm: r.get(1)?, first_beat_secs }))
    })?;
    rows.collect()
}

/// Minimal protobuf walk of `mixxx.track.io.BeatGrid`: field 2 (first_beat)
/// → field 1 (frame_position, varint).
fn first_beat_frame(blob: &[u8]) -> Option<f64> {
    let mut i = 0;
    while i < blob.len() {
        let (key, n) = varint(&blob[i..])?;
        i += n;
        match (key >> 3, key & 7) {
            (2, 2) => {
                let (len, n) = varint(&blob[i..])?;
                i += n;
                let msg = blob.get(i..i + len as usize)?;
                let mut j = 0;
                while j < msg.len() {
                    let (k, n) = varint(&msg[j..])?;
                    j += n;
                    match (k >> 3, k & 7) {
                        (1, 0) => return varint(&msg[j..]).map(|(v, _)| v as i64 as f64),
                        (_, 0) => j += varint(&msg[j..])?.1,
                        (_, 1) => j += 8,
                        (_, 5) => j += 4,
                        (_, 2) => {
                            let (l, n) = varint(&msg[j..])?;
                            j += n + l as usize;
                        }
                        _ => return None,
                    }
                }
                return None;
            }
            (_, 0) => i += varint(&blob[i..])?.1,
            (_, 1) => i += 8,
            (_, 5) => i += 4,
            (_, 2) => {
                let (l, n) = varint(&blob[i..])?;
                i += n + l as usize;
            }
            _ => return None,
        }
    }
    None
}

fn varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, byte) in b.iter().enumerate().take(10) {
        v |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}
