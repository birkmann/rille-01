//! Serato's track database on a drive (`_Serato_/database V2`): tags, BPM
//! and key for every file Serato has seen there, in one sequential read. A
//! stick prepared with Serato shows its BPMs at once, before any file is
//! opened.

use crate::meta::{FileMeta, Tags, parse_key};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The database file below a drive's root.
const DATABASE: &str = "_Serato_/database V2";

/// The root holding a Serato database at or above `dir`, if any.
pub fn serato_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|a| a.join(DATABASE).is_file()).map(Path::to_owned)
}

/// The tracks of the Serato database under `root`, by absolute path (the
/// database stores them relative to the drive's root). Covers are not in
/// the database; sizes and times are left 0.
pub fn serato_tracks(root: &Path) -> std::io::Result<HashMap<PathBuf, FileMeta>> {
    let data = std::fs::read(root.join(DATABASE))?;
    Ok(fields(&data)
        .filter(|(tag, _)| tag == b"otrk")
        .filter_map(|(_, track)| {
            let mut path = None;
            let mut t = Tags::default();
            for (tag, value) in fields(track) {
                let s = || utf16(value);
                match &tag {
                    b"pfil" => path = Some(root.join(s().replace('\\', "/").trim_start_matches('/'))),
                    b"tsng" => t.title = s(),
                    b"tart" => t.artist = s(),
                    b"talb" => t.album = s(),
                    b"tgen" => t.genre = s(),
                    b"tlbl" => t.label = s(),
                    b"tcom" => t.comment = s(),
                    b"ttyr" => t.year = s().get(..4).and_then(|y| y.parse().ok()).filter(|y| *y > 0),
                    b"tbpm" => {
                        t.bpm = s()
                            .replace(',', ".")
                            .parse()
                            .ok()
                            .filter(|b: &f64| b.is_finite() && *b > 0.0 && *b < 1000.0)
                    }
                    b"tkey" => t.key = parse_key(&s()),
                    b"tlen" => t.duration_secs = duration(&s()).unwrap_or(0.0),
                    _ => {}
                }
            }
            let path = path?;
            Some((path.clone(), FileMeta { path, size: 0, mtime: 0, tags: t, cover: None }))
        })
        .collect())
}

/// Serato's records: a 4-byte tag, a big-endian length, the value.
fn fields(mut b: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let tag: [u8; 4] = b.get(..4)?.try_into().ok()?;
        let len = u32::from_be_bytes(b.get(4..8)?.try_into().ok()?) as usize;
        let value = b.get(8..8 + len)?;
        b = &b[8 + len..];
        Some((tag, value))
    })
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units).trim_end_matches('\0').trim().to_owned()
}

/// "06:36.00" or "1:02:03.5" as seconds.
fn duration(s: &str) -> Option<f64> {
    s.split(':').try_fold(0.0, |acc, part| Some(acc * 60.0 + part.trim().parse::<f64>().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(tag: &[u8; 4], value: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        out.extend_from_slice(value);
        out
    }

    fn text(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn reads_tracks_relative_to_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut track = field(b"pfil", &text("House/01 A - B.mp3"));
        track.extend(field(b"tsng", &text("B")));
        track.extend(field(b"tart", &text("A")));
        track.extend(field(b"tbpm", &text("126.00")));
        track.extend(field(b"tkey", &text("11m")));
        track.extend(field(b"tlen", &text("06:36.00")));
        track.extend(field(b"ttyr", &text("2021")));
        let mut db = field(b"vrsn", &text("2.0/Serato Scratch LIVE Database"));
        db.extend(field(b"otrk", &track));
        db.extend(field(b"otrk", &field(b"tsng", &text("no path"))));
        std::fs::create_dir_all(dir.path().join("_Serato_")).unwrap();
        std::fs::write(dir.path().join(DATABASE), db).unwrap();

        let house = dir.path().join("House");
        std::fs::create_dir_all(&house).unwrap();
        assert_eq!(serato_root(&house).as_deref(), Some(dir.path()));
        let tracks = serato_tracks(dir.path()).unwrap();
        assert_eq!(tracks.len(), 1);
        let t = &tracks[&house.join("01 A - B.mp3")].tags;
        assert_eq!((t.title.as_str(), t.artist.as_str(), t.bpm, t.year), ("B", "A", Some(126.0), Some(2021)));
        assert_eq!(t.key, parse_key("11m"));
        assert!((t.duration_secs - 396.0).abs() < 1e-9);
    }
}
