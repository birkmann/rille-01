//! Reading a file's metadata: tags, audio properties, content hash and cover
//! thumbnails. Nothing here touches the database.

use crate::{Result, io_err};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use lofty::config::ParseOptions;
use lofty::file::{FileType, TaggedFile};
use lofty::id3::v2::Id3v2Tag;
use lofty::mpeg::MpegFile;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::Tag;
use rille_core::Key;
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
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

/// What the browser shows for a file outside the collection: tags and cover,
/// without the content hash (which costs extra reads on slow media).
#[derive(Clone, Debug, Default)]
pub struct FileMeta {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: i64,
    pub tags: Tags,
    /// Cover hash; thumbnails exist in the cache when set.
    pub cover: Option<String>,
}

/// Reads everything about `path`. Only IO errors are fatal; unreadable tags
/// fall back to the file name and are reported in `warning`.
pub fn read_file(path: &Path, cache_dir: &Path) -> Result<FileInfo> {
    let (m, warning) = read_meta_inner(path, cache_dir)?;
    let hash = content_hash(path, m.size).map_err(io_err(path))?;
    Ok(FileInfo { path: m.path, size: m.size, mtime: m.mtime, hash, tags: m.tags, cover: m.cover, warning })
}

/// Tags, audio properties and cover of `path` (no content hash). Only IO
/// errors are fatal; unreadable tags fall back to the file name.
pub fn read_meta(path: &Path, cache_dir: &Path) -> Result<FileMeta> {
    read_meta_inner(path, cache_dir).map(|(m, _)| m)
}

fn read_meta_inner(path: &Path, cache_dir: &Path) -> Result<(FileMeta, Option<String>)> {
    let (size, mtime) = stat(path).map_err(io_err(path))?;
    let mut warnings = Vec::new();
    let (mut tags, picture) = read_tags(path, ParseOptions::new()).unwrap_or_else(|e| {
        warnings.push(format!("tags: {e}"));
        (Tags::default(), None)
    });
    fill_from_filename(&mut tags, path);
    let cover = picture
        .or_else(|| folder_cover(path))
        .and_then(|bytes| make_cover(&bytes, cache_dir).map_err(|e| warnings.push(format!("cover: {e}"))).ok());
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    Ok((FileMeta { path: path.to_owned(), size, mtime, tags, cover }, warning))
}

/// What the browser shows first for a file outside the collection: tags and
/// duration, without the cover. An MP3's ID3 tag is read frame by frame,
/// seeking over pictures and other binary frames: they are most of its
/// bytes, and USB sticks are slow to read. [`read_meta`] fills in the rest.
pub fn read_meta_quick(path: &Path) -> Result<FileMeta> {
    let (size, mtime) = stat(path).map_err(io_err(path))?;
    let mut tags = match id3_quick(path, size) {
        Ok(Some(tags)) => tags,
        // Not an MP3 with an ID3v2.3/2.4 tag: lofty, without the pictures.
        _ => read_tags(path, ParseOptions::new().read_cover_art(false)).map(|(t, _)| t).unwrap_or_default(),
    };
    fill_from_filename(&mut tags, path);
    Ok(FileMeta { path: path.to_owned(), size, mtime, tags, cover: None })
}

/// ID3v2 frames a browser row uses; the rest are skipped unread.
const ROW_FRAMES: [&[u8]; 13] = [
    b"TIT2", b"TPE1", b"TPE2", b"TALB", b"TPE4", b"TPUB", b"TCON", b"COMM", b"TYER", b"TDRC", b"TBPM", b"TKEY", b"TXXX",
];
/// Bytes read per frame header lookup: small frames come in one read.
const ID3_CHUNK: u64 = 4096;
/// A larger "text" frame is not one worth waiting for.
const MAX_ROW_FRAME: u64 = 64 * 1024;

/// Tags and stream properties of an MP3 from its text frames and its first
/// audio frame. `None` for files this does not handle (no ID3v2.3/2.4 tag,
/// whole-tag unsynchronisation, a tag lofty cannot parse).
fn id3_quick(path: &Path, size: u64) -> io::Result<Option<Tags>> {
    let f = File::open(path)?;
    advise_random(&f);
    let mut head = [0u8; 10];
    read_at(&f, 0, &mut head)?;
    if &head[..3] != b"ID3" || !matches!(head[3], 3 | 4) || head[5] & 0x80 != 0 {
        return Ok(None);
    }
    let v4 = head[3] == 4;
    let tag_end = (10 + syncsafe(&head[6..10])).min(size);
    let mut pos = 10;
    if head[5] & 0x40 != 0 {
        let mut b = [0u8; 4];
        read_at(&f, pos, &mut b)?;
        pos += if v4 { syncsafe(&b) } else { 4 + u64::from(u32::from_be_bytes(b)) };
    }
    let mut kept = Vec::new();
    let (mut buf, mut buf_at) = (Vec::new(), 0);
    while pos + 10 <= tag_end {
        if pos < buf_at || pos + 10 > buf_at + buf.len() as u64 {
            buf = read_vec(&f, pos, ID3_CHUNK.min(tag_end - pos))?;
            buf_at = pos;
        }
        let h = &buf[(pos - buf_at) as usize..][..10];
        // Padding, or a size this misread: what was kept so far stands.
        if !h[..4].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            break;
        }
        let len = if v4 { syncsafe(&h[4..8]) } else { u64::from(u32::from_be_bytes([h[4], h[5], h[6], h[7]])) };
        let end = pos + 10 + len;
        if end > tag_end {
            break;
        }
        if len <= MAX_ROW_FRAME && ROW_FRAMES.contains(&&h[..4]) {
            if end > buf_at + buf.len() as u64 {
                buf = read_vec(&f, pos, end - pos)?;
                buf_at = pos;
            }
            kept.extend_from_slice(&buf[(pos - buf_at) as usize..(end - buf_at) as usize]);
        }
        pos = end;
    }
    let audio = read_vec(&f, tag_end, ID3_CHUNK.min(size - tag_end))?;
    // A tag of only the kept frames, then the first audio bytes, for lofty.
    let kept_len = u32::try_from(kept.len()).expect("frames under 4 GiB");
    let mut mini = Vec::with_capacity(10 + kept.len() + audio.len());
    mini.extend_from_slice(&[b'I', b'D', b'3', head[3], 0, 0]);
    mini.extend_from_slice(&[
        (kept_len >> 21) as u8 & 0x7f,
        (kept_len >> 14) as u8 & 0x7f,
        (kept_len >> 7) as u8 & 0x7f,
        kept_len as u8 & 0x7f,
    ]);
    mini.extend_from_slice(&kept);
    mini.extend_from_slice(&audio);
    let options = ParseOptions::new().read_properties(false).read_cover_art(false);
    let Ok(file) = Probe::new(Cursor::new(&mini)).options(options).set_file_type(FileType::Mpeg).read() else {
        return Ok(None);
    };
    let (mut tags, _) = tags_of(&file);
    if let Ok(mp3) = MpegFile::read_from(&mut Cursor::new(&mini), options)
        && let Some(id3) = mp3.id3v2()
    {
        user_texts(id3, &mut tags);
    }
    if let Some(s) = mpeg_stream(&audio, size - tag_end) {
        (tags.duration_secs, tags.bitrate, tags.sample_rate) = (s.duration_secs, Some(s.kbps), Some(s.sample_rate));
    }
    Ok(Some(tags))
}

fn syncsafe(b: &[u8]) -> u64 {
    b.iter().fold(0, |n, &x| n << 7 | u64::from(x & 0x7f))
}

fn read_at(f: &File, pos: u64, buf: &mut [u8]) -> io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::FileExt::read_exact_at(f, buf, pos);
    #[cfg(not(unix))]
    {
        let mut f = f;
        f.seek(SeekFrom::Start(pos))?;
        f.read_exact(buf)
    }
}

fn read_vec(f: &File, pos: u64, len: u64) -> io::Result<Vec<u8>> {
    let mut buf = vec![0; len as usize];
    read_at(f, pos, &mut buf)?;
    Ok(buf)
}

/// Turns off read-ahead, which would pull in the pictures being skipped.
fn advise_random(f: &File) {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: a valid open descriptor; the call only sets a hint.
        unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_RANDOM) };
    }
    #[cfg(not(target_os = "linux"))]
    let _ = f;
}

#[derive(Debug, PartialEq)]
struct MpegStream {
    duration_secs: f64,
    kbps: u32,
    sample_rate: u32,
}

/// Duration, bitrate and sample rate from the first MPEG layer III frame in
/// `head`: a Xing/Info header's frame count, else the bitrate over
/// `stream_len` (constant bitrate).
fn mpeg_stream(head: &[u8], stream_len: u64) -> Option<MpegStream> {
    const KBPS_V1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const KBPS_V2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let (i, version, kbps, sample_rate) = head.windows(4).enumerate().find_map(|(i, h)| {
        // Sync, layer III, a valid bitrate and sample rate.
        if h[0] != 0xff || h[1] & 0xe0 != 0xe0 || h[1] & 0x06 != 0x02 {
            return None;
        }
        let version = (h[1] >> 3) & 3; // 0: 2.5, 2: 2, 3: 1
        let (b, r) = ((h[2] >> 4) as usize, ((h[2] >> 2) & 3) as usize);
        if version == 1 || b == 0 || b == 15 || r == 3 {
            return None;
        }
        let kbps = if version == 3 { KBPS_V1[b] } else { KBPS_V2[b] };
        let rate = [44100, 48000, 32000][r] >> (3 - version.max(1));
        Some((i, version, kbps, rate))
    })?;
    let mono = head[i + 3] >> 6 == 3;
    let side_info = match (version == 3, mono) {
        (true, false) => 32,
        (true, true) | (false, false) => 17,
        (false, true) => 9,
    };
    let samples_per_frame = if version == 3 { 1152.0 } else { 576.0 };
    let xing = head.get(i + 4 + side_info..i + 4 + side_info + 12).filter(|x| &x[..4] == b"Xing" || &x[..4] == b"Info");
    let frames = xing.filter(|x| x[7] & 1 != 0).map(|x| u32::from_be_bytes([x[8], x[9], x[10], x[11]]));
    let duration_secs = match frames {
        Some(n) => f64::from(n) * samples_per_frame / f64::from(sample_rate),
        None => stream_len.saturating_sub(i as u64) as f64 * 8.0 / (f64::from(kbps) * 1000.0),
    };
    Some(MpegStream { duration_secs, kbps, sample_rate })
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

fn read_tags(path: &Path, options: ParseOptions) -> Result<(Tags, Option<Vec<u8>>), lofty::error::FileParseError> {
    // Sniff the content: extensions lie, and ".alac" is not known by name.
    let file = Probe::open(path)?.options(options).guess_file_type()?.read()?;
    let (mut tags, picture) = tags_of(&file);
    if file.file_type() == FileType::Mpeg && (tags.label.is_empty() || tags.key.is_none()) {
        id3_user_texts(path, &mut tags);
    }
    Ok((tags, picture))
}

/// A parsed file's tags and its cover image (front cover first).
fn tags_of(file: &TaggedFile) -> (Tags, Option<Vec<u8>>) {
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
    let pics = || all.iter().flat_map(|t| t.pictures());
    let picture = pics().find(|p| p.pic_type() == PictureType::CoverFront).or_else(|| pics().next());
    (tags, picture.map(|p| p.data().to_vec()))
}

/// Beatport and Traktor write label and key into TXXX frames, which lofty's
/// generic tag drops; read them from the ID3v2 tag directly.
fn id3_user_texts(path: &Path, tags: &mut Tags) {
    let Ok(mut f) = File::open(path) else { return };
    let Ok(mp3) = MpegFile::read_from(&mut f, ParseOptions::new().read_properties(false).read_cover_art(false)) else {
        return;
    };
    if let Some(id3) = mp3.id3v2() {
        user_texts(id3, tags);
    }
}

fn user_texts(id3: &Id3v2Tag, tags: &mut Tags) {
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
pub(crate) fn make_cover(bytes: &[u8], cache_dir: &Path) -> Result<String, String> {
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

    #[test]
    fn quick_read_skips_the_cover_and_matches_the_full_read() {
        use lofty::config::WriteOptions;
        use lofty::picture::{MimeType, Picture};
        use lofty::tag::TagType;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("01 x.mp3");
        // 100 silent MPEG-1 layer III frames, 128 kbit/s, 44.1 kHz.
        let frame: Vec<u8> = [0xff, 0xfb, 0x90, 0x00].into_iter().chain(std::iter::repeat_n(0, 413)).collect();
        std::fs::write(&path, frame.repeat(100)).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.push_picture(
            Picture::unchecked(vec![7; 300_000]).pic_type(PictureType::CoverFront).mime_type(MimeType::Png).build(),
        );
        tag.insert_text(ItemKey::TrackTitle, "Title".into());
        tag.insert_text(ItemKey::TrackArtist, "Artist".into());
        tag.insert_text(ItemKey::Label, "Label".into());
        tag.insert_text(ItemKey::IntegerBpm, "126".into());
        tag.insert_text(ItemKey::InitialKey, "8A".into());
        tag.save_to_path(&path, WriteOptions::default()).unwrap();

        let quick = read_meta_quick(&path).unwrap();
        let full = read_meta(&path, dir.path()).unwrap();
        let (q, f) = (&quick.tags, &full.tags);
        assert_eq!((q.title.as_str(), q.artist.as_str(), q.label.as_str()), ("Title", "Artist", "Label"));
        assert_eq!((q.bpm, q.key), (Some(126.0), parse_key("8A")));
        assert_eq!((q.bpm, q.key, &q.title, &q.label), (f.bpm, f.key, &f.title, &f.label));
        assert_eq!((q.bitrate, q.sample_rate), (Some(128), Some(44100)));
        assert!((q.duration_secs - 2.6).abs() < 0.05, "{}", q.duration_secs);
        assert!(quick.cover.is_none());
    }

    #[test]
    fn mpeg_stream_reads_xing_frame_counts() {
        // MPEG-1 layer III, 44.1 kHz, stereo: Xing after 32 bytes of side info.
        let mut head = vec![0x00, 0x11, 0xff, 0xfb, 0x90, 0x00];
        head.extend([0; 32]);
        head.extend(b"Xing");
        head.extend(1u32.to_be_bytes());
        head.extend(1000u32.to_be_bytes());
        let s = mpeg_stream(&head, 1 << 20).unwrap();
        assert_eq!((s.kbps, s.sample_rate), (128, 44100));
        assert!((s.duration_secs - 1000.0 * 1152.0 / 44100.0).abs() < 1e-9);
        assert_eq!(mpeg_stream(&[0xff, 0xfb, 0xf0, 0x00], 1000), None, "bitrate index 15 is invalid");
    }
}
