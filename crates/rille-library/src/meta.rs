//! Reading a file's metadata: tags, audio properties, content hash and cover
//! thumbnails. Nothing here touches the database.

use crate::{Result, io_err};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use lofty::config::ParseOptions;
use lofty::file::FileType;
use lofty::mpeg::MpegFile;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::Tag;
use rille_core::Key;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use xxhash_rust::xxh3::Xxh3;

pub const SUPPORTED_EXTENSIONS: &[&str] =
    &["mp3", "flac", "wav", "aiff", "aif", "ogg", "opus", "m4a", "mp4", "aac", "alac"];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SUPPORTED_EXTENSIONS.iter().any(|s| s.eq_ignore_ascii_case(e)))
}

/// Cover thumbnail edge lengths in pixels.
pub const COVER_SIZES: [u32; 2] = [64, 256];

/// Tag and stream information. Text fields are empty when absent.
#[derive(Clone, Debug, Default)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub remixer: String,
    pub label: String,
    pub genre: String,
    pub comment: String,
    pub year: Option<i32>,
    pub bpm: Option<f64>,
    pub key: Option<Key>,
    pub duration_secs: f64,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
}

/// Everything the library stores about a file on import.
#[derive(Debug)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: i64,
    pub hash: u64,
    pub tags: Tags,
    /// Cover hash; thumbnails exist in the cache when set.
    pub cover: Option<String>,
    /// Non-fatal problem (unreadable tags or cover); the file is still imported.
    pub warning: Option<String>,
}

/// `(size, mtime)` of a file.
pub fn stat(path: &Path) -> io::Result<(u64, i64)> {
    let m = std::fs::metadata(path)?;
    let mtime = m.modified()?.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    Ok((m.len(), mtime))
}

/// Reads everything about `path`. Only IO errors are fatal; unreadable tags
/// fall back to the file name and are reported in `warning`.
pub fn read_file(path: &Path, cache_dir: &Path) -> Result<FileInfo> {
    let (size, mtime) = stat(path).map_err(io_err(path))?;
    let hash = content_hash(path, size).map_err(io_err(path))?;
    let mut warnings = Vec::new();
    let (mut tags, picture) = read_tags(path).unwrap_or_else(|e| {
        warnings.push(format!("tags: {e}"));
        (Tags::default(), None)
    });
    fill_from_filename(&mut tags, path);
    let cover = picture
        .or_else(|| folder_cover(path))
        .and_then(|bytes| make_cover(&bytes, cache_dir).map_err(|e| warnings.push(format!("cover: {e}"))).ok());
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    Ok(FileInfo { path: path.to_owned(), size, mtime, hash, tags, cover, warning })
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

fn read_tags(path: &Path) -> Result<(Tags, Option<Vec<u8>>), lofty::error::FileParseError> {
    // Sniff the content: extensions lie, and ".alac" is not known by name.
    let file = Probe::open(path)?.guess_file_type()?.read()?;
    // Primary tag first, then any others (e.g. ID3v1, or ID3v2 in a FLAC).
    let mut all: Vec<&Tag> = file.primary_tag().into_iter().collect();
    all.extend(file.tags().iter().filter(|t| Some(t.tag_type()) != file.primary_tag().map(|p| p.tag_type())));

    let text = |keys: &[ItemKey]| {
        keys.iter()
            .find_map(|k| all.iter().find_map(|t| t.get_string(*k).map(str::trim).filter(|s| !s.is_empty())))
            .unwrap_or_default()
            .to_owned()
    };
    let year = all.iter().find_map(|t| t.date()).map(|d| i32::from(d.year)).or_else(|| {
        let s = text(&[ItemKey::Year, ItemKey::RecordingDate, ItemKey::OriginalReleaseDate]);
        s.get(..4)?.parse().ok()
    });
    let bpm = text(&[ItemKey::Bpm, ItemKey::IntegerBpm]).replace(',', ".").parse::<f64>().ok();
    let props = file.properties();
    let tags = Tags {
        title: text(&[ItemKey::TrackTitle]),
        artist: text(&[ItemKey::TrackArtist, ItemKey::AlbumArtist]),
        album: text(&[ItemKey::AlbumTitle]),
        remixer: text(&[ItemKey::Remixer]),
        label: text(&[ItemKey::Label, ItemKey::Publisher]),
        genre: text(&[ItemKey::Genre]),
        comment: text(&[ItemKey::Comment]),
        year: year.filter(|y| *y > 0),
        bpm: bpm.filter(|b| b.is_finite() && *b > 0.0 && *b < 1000.0),
        key: parse_key(&text(&[ItemKey::InitialKey])),
        duration_secs: props.duration().as_secs_f64(),
        bitrate: props.audio_bitrate().or(props.overall_bitrate()).filter(|b| *b > 0),
        sample_rate: props.sample_rate().filter(|r| *r > 0),
    };
    let mut tags = tags;
    if file.file_type() == FileType::Mpeg && (tags.label.is_empty() || tags.key.is_none()) {
        id3_user_texts(path, &mut tags);
    }
    let pics = || all.iter().flat_map(|t| t.pictures());
    let picture = pics().find(|p| p.pic_type() == PictureType::CoverFront).or_else(|| pics().next());
    Ok((tags, picture.map(|p| p.data().to_vec())))
}

/// Beatport and Traktor write label and key into TXXX frames, which lofty's
/// generic tag drops; read them from the ID3v2 tag directly.
fn id3_user_texts(path: &Path, tags: &mut Tags) {
    let Ok(mut f) = File::open(path) else { return };
    let Ok(mp3) = MpegFile::read_from(&mut f, ParseOptions::new().read_properties(false).read_cover_art(false)) else {
        return;
    };
    let Some(id3) = mp3.id3v2() else { return };
    let get = |keys: &[&str]| keys.iter().find_map(|k| id3.get_user_text(k)).map(str::trim).filter(|s| !s.is_empty());
    if let (true, Some(label)) = (tags.label.is_empty(), get(&["LABEL", "PUBLISHER"])) {
        label.clone_into(&mut tags.label);
    }
    if tags.key.is_none() {
        tags.key = get(&["INITIALKEY", "KEY"]).and_then(parse_key);
    }
}

/// Fills a missing title/artist from a file name like "01. Artist - Title".
fn fill_from_filename(tags: &mut Tags, path: &Path) {
    let stem = path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
    let stem = strip_track_number(stem.trim());
    let (artist, title) = match stem.split_once(" - ") {
        Some((a, t)) if !a.trim().is_empty() && !t.trim().is_empty() => (Some(a.trim()), t.trim()),
        _ => (None, stem),
    };
    if tags.title.is_empty() {
        title.clone_into(&mut tags.title);
    }
    if let (true, Some(a)) = (tags.artist.is_empty(), artist) {
        a.clone_into(&mut tags.artist);
    }
}

/// "01. X", "01 - X", "01-X", "01_X"; a bare space only after exactly two
/// digits so "808 State" survives.
fn strip_track_number(s: &str) -> &str {
    let digits = s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || digits > 3 {
        return s;
    }
    let rest = &s[digits..];
    let sep = [". ", " - ", ".", "-", "_"].iter().find(|p| rest.starts_with(**p));
    let stripped = match sep {
        Some(p) => &rest[p.len()..],
        None if digits == 2 && rest.starts_with(' ') => &rest[1..],
        None => return s,
    };
    let stripped = stripped.trim_start();
    if stripped.is_empty() { s } else { stripped }
}

/// Parses key tags in musical ("Am", "F#", "Bbmin"), Camelot ("8A") or Open
/// Key ("1m") notation.
pub fn parse_key(s: &str) -> Option<Key> {
    let s = s.trim();
    let lower = s.to_lowercase();
    let digits = lower.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if let Ok(n) = digits.parse::<u8>() {
        if !(1..=12).contains(&n) {
            return None;
        }
        let (camelot, minor) = match &lower[digits.len()..] {
            "a" => (n, true),
            "b" => (n, false),
            "m" => ((n + 6) % 12 + 1, true),
            "d" => ((n + 6) % 12 + 1, false),
            _ => return None,
        };
        return (0..24u8)
            .filter_map(|v| Key::try_from(v).ok())
            .find(|k| k.camelot_number() == camelot && k.is_minor() == minor);
    }
    let mut chars = lower.chars();
    let base: i8 = match chars.next()? {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (acc, rest) = match rest.chars().next() {
        Some(c @ ('#' | '♯' | 'b' | '♭')) => (if matches!(c, '#' | '♯') { 1 } else { -1 }, &rest[c.len_utf8()..]),
        _ => (0, rest),
    };
    let minor = match rest.trim() {
        "" | "maj" | "major" => false,
        "m" | "min" | "minor" => true,
        _ => return None,
    };
    Some(Key::new((base + acc).rem_euclid(12) as u8, minor))
}

// ---------------------------------------------------------------------------
// Content hash
// ---------------------------------------------------------------------------

const HASH_CHUNK: u64 = 64 * 1024;

/// xxh3 over the size and three 64 KiB samples (start, middle, end). Cheap
/// enough for 10k-file scans and stable across moves and renames.
pub fn content_hash(path: &Path, size: u64) -> io::Result<u64> {
    let mut f = File::open(path)?;
    let mut h = Xxh3::new();
    h.update(&size.to_le_bytes());
    let mut buf = vec![0; HASH_CHUNK as usize];
    if size <= 3 * HASH_CHUNK {
        let mut all = Vec::with_capacity(size as usize);
        f.read_to_end(&mut all)?;
        h.update(&all);
    } else {
        for off in [0, size / 2 - HASH_CHUNK / 2, size - HASH_CHUNK] {
            f.seek(SeekFrom::Start(off))?;
            f.read_exact(&mut buf)?;
            h.update(&buf);
        }
    }
    Ok(h.digest())
}

// ---------------------------------------------------------------------------
// Covers
// ---------------------------------------------------------------------------

pub fn cover_file(cache_dir: &Path, cover: &str, px: u32) -> PathBuf {
    cache_dir.join("covers").join(format!("{cover}_{px}.jpg"))
}

fn folder_cover(track: &Path) -> Option<Vec<u8>> {
    const NAMES: [&str; 7] =
        ["cover.jpg", "folder.jpg", "front.jpg", "Cover.jpg", "Folder.jpg", "cover.png", "folder.png"];
    let dir = track.parent()?;
    NAMES.iter().find_map(|n| std::fs::read(dir.join(n)).ok())
}

/// Writes JPEG thumbnails for an image (deduplicated by content hash) and
/// returns the hash.
fn make_cover(bytes: &[u8], cache_dir: &Path) -> Result<String, String> {
    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let cover = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(bytes));
    if COVER_SIZES.iter().all(|px| cover_file(cache_dir, &cover, *px).exists()) {
        return Ok(cover);
    }
    let mut img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(cache_dir.join("covers")).map_err(|e| e.to_string())?;
    // Largest first, so the small one is scaled from an already small image.
    for px in COVER_SIZES.into_iter().rev() {
        img = img.resize_to_fill(px, px, FilterType::Triangle);
        let rgb = image::DynamicImage::ImageRgb8(img.to_rgb8());
        let mut jpeg = Vec::new();
        rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut jpeg, 85)).map_err(|e| e.to_string())?;
        // Write-then-rename: parallel scans may produce the same cover.
        let dest = cover_file(cache_dir, &cover, px);
        let tmp = dest.with_extension(format!("{}.tmp", TMP_COUNTER.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&tmp, &jpeg).and_then(|()| std::fs::rename(&tmp, &dest)).map_err(|e| e.to_string())?;
    }
    Ok(cover)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(parse_key("Am"), Some(Key::new(9, true)));
        assert_eq!(parse_key("8A"), Some(Key::new(9, true)));
        assert_eq!(parse_key("08a"), Some(Key::new(9, true)));
        assert_eq!(parse_key("1m"), Some(Key::new(9, true)));
        assert_eq!(parse_key("1d"), Some(Key::new(0, false)));
        assert_eq!(parse_key("F#m"), Some(Key::new(6, true)));
        assert_eq!(parse_key("Bbmin"), Some(Key::new(10, true)));
        assert_eq!(parse_key("Db"), Some(Key::new(1, false)));
        assert_eq!(parse_key("B"), Some(Key::new(11, false)));
        assert_eq!(parse_key("Cb"), Some(Key::new(11, false)));
        for bad in ["", "13A", "H", "Am7", "0A"] {
            assert_eq!(parse_key(bad), None, "{bad}");
        }
        for v in 0..24u8 {
            let k = Key::try_from(v).unwrap();
            assert_eq!(parse_key(&k.camelot()), Some(k));
            assert_eq!(parse_key(&k.open_key()), Some(k));
            assert_eq!(parse_key(&k.musical()), Some(k));
        }
    }

    #[test]
    fn filename_fallback() {
        let cases = [
            ("01. Easy Peelers - Polymorph (Original Mix).mp3", "Easy Peelers", "Polymorph (Original Mix)"),
            ("00 - Adventice - She Pulled Out the Weeds.flac", "Adventice", "She Pulled Out the Weeds"),
            ("808 State - Pacific.mp3", "808 State", "Pacific"),
            ("12 Intro.wav", "", "Intro"),
            ("just a title.wav", "", "just a title"),
            ("2024.wav", "", "2024"),
        ];
        for (name, artist, title) in cases {
            let mut t = Tags::default();
            fill_from_filename(&mut t, Path::new(name));
            assert_eq!((t.artist.as_str(), t.title.as_str()), (artist, title), "{name}");
        }
        let mut t = Tags { title: "Tagged".into(), ..Tags::default() };
        fill_from_filename(&mut t, Path::new("A - B.mp3"));
        assert_eq!((t.artist.as_str(), t.title.as_str()), ("A", "Tagged"));
    }
}
