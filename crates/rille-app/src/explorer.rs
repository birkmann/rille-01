//! The file explorer: places to start from, folders and the audio files in
//! them. Works on the file system directly; the collection is joined in by
//! [`App::folder_rows`](crate::App::folder_rows).

use std::path::{Path, PathBuf};

use rille_library::{TrackRow, is_supported};

/// A starting point in the explorer tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    pub kind: PlaceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Music,
    /// A mounted drive (USB stick, second disk).
    Drive,
    Root,
}

/// Home, the XDG music folder, mounted drives and `/`.
pub fn places() -> Vec<Place> {
    let mut out = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(h) = &home {
        out.push(Place { name: "Home".into(), path: h.clone(), kind: PlaceKind::Home });
        let music = xdg_music_dir(h).unwrap_or_else(|| h.join("Music"));
        if music.is_dir() {
            out.push(Place { name: "Music".into(), path: music, kind: PlaceKind::Music });
        }
    }
    let user = std::env::var("USER").unwrap_or_default();
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    for dir in mount_points(&mounts, &user) {
        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        out.push(Place { name, path: dir, kind: PlaceKind::Drive });
    }
    out.push(Place { name: "Computer".into(), path: PathBuf::from("/"), kind: PlaceKind::Root });
    out
}

/// `XDG_MUSIC_DIR` from `~/.config/user-dirs.dirs`.
fn xdg_music_dir(home: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).ok()?;
    let line = text.lines().find(|l| l.trim_start().starts_with("XDG_MUSIC_DIR="))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let path = value.replace("$HOME", &home.to_string_lossy());
    Some(PathBuf::from(path)).filter(|p| p.is_absolute() && p != home)
}

/// Removable and extra drives from a `/proc/self/mounts` text: anything
/// under `/run/media/<user>`, `/media` or `/mnt`. Octal escapes (`\040`
/// for a space) are decoded.
fn mount_points(mounts: &str, user: &str) -> Vec<PathBuf> {
    let media = format!("/run/media/{user}/");
    let mut out: Vec<PathBuf> = mounts
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(unescape_mount)
        .filter(|p| (!user.is_empty() && p.starts_with(&media)) || p.starts_with("/media/") || p.starts_with("/mnt/"))
        .map(PathBuf::from)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hidden(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.as_encoded_bytes().starts_with(b"."))
}

/// Subfolders of `dir`, hidden ones left out, sorted by name (case-insensitive).
pub fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !hidden(p)).collect();
    sort_by_name(&mut out);
    out
}

/// Whether `dir` has any subfolder (for the tree's expand arrow), without
/// listing all of them.
pub fn has_subfolders(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| !hidden(&e.path()) && e.path().is_dir()))
}

/// Supported audio files in `dir` (and below with `recursive`), sorted.
pub fn audio_files(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(dir, recursive, &mut out, 0);
    sort_by_name(&mut out);
    out
}

/// A folder's audio files and the subfolders with audio somewhere below,
/// for importing a folder as playlists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderTree {
    pub name: String,
    pub files: Vec<PathBuf>,
    pub children: Vec<FolderTree>,
}

impl FolderTree {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.children.is_empty()
    }

    /// Every file, this folder's first, then each subfolder's.
    pub fn all_files(&self) -> Vec<PathBuf> {
        let mut out = self.files.clone();
        for c in &self.children {
            out.extend(c.all_files());
        }
        out
    }
}

/// `dir` as a [`FolderTree`]; subfolders only with `recursive`.
pub fn folder_tree(dir: &Path, recursive: bool) -> FolderTree {
    fn build(dir: &Path, recursive: bool, depth: usize) -> FolderTree {
        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        let children = if recursive && depth < 32 {
            subfolders(dir).iter().map(|d| build(d, true, depth + 1)).filter(|t| !t.is_empty()).collect()
        } else {
            Vec::new()
        };
        FolderTree { name, files: audio_files(dir, false), children }
    }
    build(dir, recursive, 0)
}

fn collect(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if hidden(&p) {
            continue;
        }
        if p.is_dir() {
            if recursive && depth < 32 {
                collect(&p, recursive, out, depth + 1);
            }
        } else if is_supported(&p) {
            out.push(p);
        }
    }
}

fn sort_by_name(v: &mut [PathBuf]) {
    v.sort_by_cached_key(|p| p.to_string_lossy().to_lowercase());
}

/// A browser row for a file that is not in the collection: id −1, title
/// from the file name ("Artist - Title" splits into both).
pub fn file_row(path: &Path) -> TrackRow {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // Drop a leading track number ("01. ", "07 - "), not a title like "1998".
    let stem_clean = match stem.find(|c: char| !c.is_ascii_digit()) {
        Some(i @ 1..=3)
            if stem[i..].starts_with(['.', '-', '_'])
                || (stem[i..].starts_with(' ') && (i <= 2 || stem.starts_with('0'))) =>
        {
            let rest = stem[i..].trim_start_matches(['.', ' ', '-', '_']);
            if rest.is_empty() { stem.clone() } else { rest.to_string() }
        }
        _ => stem.clone(),
    };
    let (artist, title) = match stem_clean.split_once(" - ") {
        Some((a, t)) => (a.trim().to_string(), t.trim().to_string()),
        None => (String::new(), stem_clean.clone()),
    };
    TrackRow {
        id: -1,
        path: path.to_owned(),
        title,
        artist,
        album: String::new(),
        remixer: String::new(),
        label: String::new(),
        genre: String::new(),
        comment: String::new(),
        year: None,
        duration_secs: 0.0,
        bpm: None,
        key: None,
        rating: 0,
        color: None,
        play_count: 0,
        last_played: None,
        date_added: 0,
        file_size: std::fs::metadata(path).map_or(0, |m| m.len()),
        bitrate: None,
        sample_rate: None,
        has_cover: false,
        cover: None,
        grid_confidence: None,
        grid_flags: 0,
        grid_locked: false,
        analyzed: false,
        analysis_version: None,
        analysis_failed: false,
        missing: false,
        beatport_id: None,
        beatport_offline: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drives_from_mounts() {
        let mounts = "/dev/sda1 / ext4 rw 0 0\n\
                      /dev/sdb1 /run/media/dj/USB\\040STICK vfat rw 0 0\n\
                      /dev/sdc1 /run/media/other/X vfat rw 0 0\n\
                      /dev/sdd1 /mnt/archive ext4 rw 0 0\n\
                      tmpfs /run/user/1000 tmpfs rw 0 0\n";
        assert_eq!(
            mount_points(mounts, "dj"),
            [PathBuf::from("/mnt/archive"), PathBuf::from("/run/media/dj/USB STICK")]
        );
    }

    #[test]
    fn file_rows_take_names_apart() {
        let r = file_row(Path::new("/x/01. Roland Klinkenberg - Dubcraze (Original mix).mp3"));
        assert_eq!((r.id, r.artist.as_str(), r.title.as_str()), (-1, "Roland Klinkenberg", "Dubcraze (Original mix)"));
        let r = file_row(Path::new("/x/untitled loop.wav"));
        assert_eq!((r.artist.as_str(), r.title.as_str()), ("", "untitled loop"));
        // A number is only a track number when a name follows it.
        assert_eq!(file_row(Path::new("/x/1998.flac")).title, "1998");
        assert_eq!(file_row(Path::new("/x/808 State - Pacific.mp3")).artist, "808 State");
    }

    #[test]
    fn folders_and_audio_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("b sub/deeper")).unwrap();
        std::fs::create_dir_all(d.join("A sub")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        for f in ["z.mp3", "a.FLAC", "notes.txt", "b sub/x.wav", "b sub/deeper/y.aiff", ".hidden/h.mp3"] {
            std::fs::write(d.join(f), b"").unwrap();
        }
        let names = |v: Vec<PathBuf>| -> Vec<String> {
            v.iter().map(|p| p.strip_prefix(d).unwrap().to_string_lossy().into_owned()).collect()
        };
        assert_eq!(names(subfolders(d)), ["A sub", "b sub"]);
        assert!(has_subfolders(d) && !has_subfolders(&d.join("A sub")));
        assert_eq!(names(audio_files(d, false)), ["a.FLAC", "z.mp3"]);
        assert_eq!(names(audio_files(d, true)), ["a.FLAC", "b sub/deeper/y.aiff", "b sub/x.wav", "z.mp3"]);

        // "A sub" holds no audio and is left out.
        let tree = folder_tree(d, true);
        assert_eq!(tree.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["b sub"]);
        assert_eq!(tree.children[0].children[0].name, "deeper");
        assert_eq!(names(tree.all_files()), ["a.FLAC", "z.mp3", "b sub/x.wav", "b sub/deeper/y.aiff"]);
        assert!(folder_tree(d, false).children.is_empty());
    }
}
