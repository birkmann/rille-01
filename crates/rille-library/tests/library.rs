//! Library integration tests on generated WAV fixtures in temp dirs.

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{Tag, TagType};
use rille_core::track::ANALYZER_VERSION;
use rille_core::{BeatGrid, BeatMap, CueKind, CuePoint, GridFlags, GridSource, Key, TrackAnalysis, TrackCues};
use rille_library::{CoverSize, Error, Library, ScanProgress};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct Env {
    dir: TempDir,
    lib: Library,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&dir.path().join("db/library.sqlite"), &dir.path().join("cache")).unwrap();
        std::fs::create_dir_all(dir.path().join("music")).unwrap();
        Self { dir, lib }
    }

    fn music(&self) -> PathBuf {
        self.dir.path().join("music")
    }
}

/// A stereo 16-bit sine. `secs` changes the size, so rewriting with another
/// length counts as a content change.
fn write_wav(path: &Path, secs: f32, freq: f32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..(secs * 44100.0) as usize {
        let s = ((i as f32 * freq * std::f32::consts::TAU / 44100.0).sin() * 8000.0) as i16;
        w.write_sample(s).unwrap();
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 128]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn write_tags(path: &Path) {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title("Tagged Title".into());
    tag.set_artist("Tagged Artist".into());
    tag.set_album("An Album".into());
    tag.set_genre("Techno".into());
    tag.set_comment("great for peak time".into());
    tag.insert_text(ItemKey::Remixer, "Some Remixer".into());
    tag.insert_text(ItemKey::Label, "Some Label".into());
    tag.insert_text(ItemKey::RecordingDate, "2021-05-01".into());
    tag.insert_text(ItemKey::IntegerBpm, "128".into());
    tag.insert_text(ItemKey::InitialKey, "Am".into());
    tag.push_picture(
        Picture::unchecked(png(300, 200)).pic_type(PictureType::CoverFront).mime_type(MimeType::Png).build(),
    );
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

fn analysis(bpm: f64, version: u32) -> TrackAnalysis {
    TrackAnalysis {
        analyzer_version: version,
        duration_secs: 2.0,
        sample_rate: 44100,
        grid: Some(BeatGrid::new(
            BeatMap::constant(0.05, bpm).unwrap(),
            GridSource::Auto { analyzer_version: version },
        )),
        key: Some(Key::new(9, true)),
        key_confidence: 0.8,
        lufs: Some(-9.5),
        peak_db: Some(-0.3),
        first_sound_secs: 0.01,
    }
}

#[test]
fn schema_create_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lib.sqlite");
    let cache = dir.path().join("cache");
    std::fs::create_dir(dir.path().join("root")).unwrap();
    {
        let mut lib = Library::open(&db, &cache).unwrap();
        lib.add_root(&dir.path().join("root")).unwrap();
        lib.add_root(&dir.path().join("root")).unwrap();
        lib.create_playlist(None, "Keep me", false).unwrap();
    }
    for _ in 0..2 {
        let lib = Library::open(&db, &cache).unwrap();
        assert_eq!(lib.roots().unwrap(), [dir.path().join("root").canonicalize().unwrap()]);
        assert_eq!(lib.playlist_tree().unwrap()[0].name, "Keep me");
    }
    let conn = rusqlite::Connection::open(&db).unwrap();
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(version, 2);
    let mode: String = conn.pragma_query_value(None, "journal_mode", |r| r.get(0)).unwrap();
    assert_eq!(mode, "wal");

    // A database from a newer version is refused rather than corrupted.
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    assert!(matches!(Library::open(&db, &cache), Err(Error::Invalid(_))));

    let mut lib = Library::open_in_memory(&cache).unwrap();
    assert!(lib.tracks().unwrap().is_empty());
    assert!(lib.add_root(&dir.path().join("nope")).is_err());
}

#[test]
fn import_untagged_wav_uses_file_name() {
    let mut env = Env::new();
    let path = env.music().join("03 - Some Artist - Some Title.wav");
    write_wav(&path, 2.0, 440.0);
    let id = env.lib.import_file(&path).unwrap();
    assert_eq!(env.lib.import_file(&path).unwrap(), id, "re-import is idempotent");
    assert_eq!(env.lib.track_by_path(&path).unwrap(), Some(id));

    let t = env.lib.track(id).unwrap().unwrap();
    assert_eq!((t.title.as_str(), t.artist.as_str()), ("Some Title", "Some Artist"));
    assert!((t.duration_secs - 2.0).abs() < 0.01, "{}", t.duration_secs);
    assert_eq!(t.sample_rate, Some(44100));
    assert_eq!(t.file_size, std::fs::metadata(&path).unwrap().len());
    assert!(!t.has_cover && !t.analyzed && !t.missing);
    assert!(t.date_added > 0);
    assert_eq!(env.lib.cover_path(id, CoverSize::Small), None);

    let txt = env.music().join("notes.txt");
    std::fs::write(&txt, "hi").unwrap();
    assert!(matches!(env.lib.import_file(&txt), Err(Error::Unsupported(_))));
}

#[test]
fn import_tagged_wav_with_cover() {
    let mut env = Env::new();
    let path = env.music().join("whatever.wav");
    write_wav(&path, 1.0, 440.0);
    write_tags(&path);
    let id = env.lib.import_file(&path).unwrap();
    let t = env.lib.track(id).unwrap().unwrap();
    assert_eq!(t.title, "Tagged Title");
    assert_eq!(t.artist, "Tagged Artist");
    assert_eq!(t.album, "An Album");
    assert_eq!(t.genre, "Techno");
    assert_eq!(t.comment, "great for peak time");
    assert_eq!(t.remixer, "Some Remixer");
    assert_eq!(t.label, "Some Label");
    assert_eq!(t.year, Some(2021));
    assert_eq!(t.bpm, Some(128.0));
    assert_eq!(t.key, Some(Key::new(9, true)));
    assert!(t.has_cover);
    for size in [CoverSize::Small, CoverSize::Large] {
        let p = env.lib.cover_path(id, size).unwrap();
        assert!(p.starts_with(env.dir.path().join("cache/covers")));
        let img = image::open(&p).unwrap();
        assert_eq!((img.width(), img.height()), (size.px(), size.px()));
    }
}

#[test]
fn scan_adds_updates_relinks_and_marks_missing() {
    let mut env = Env::new();
    let root = env.music();
    write_wav(&root.join("a/one.wav"), 1.0, 220.0);
    write_wav(&root.join("a/two.wav"), 1.2, 330.0);
    write_wav(&root.join("b/three.wav"), 1.4, 440.0);
    std::fs::write(root.join("b/readme.txt"), "skip me").unwrap();
    std::fs::write(root.join("b/broken.mp3"), b"definitely not an mp3").unwrap();
    write_wav(&root.join(".hidden/four.wav"), 1.0, 550.0);
    env.lib.add_root(&root).unwrap();

    let mut events = Vec::new();
    let r = env.lib.scan(&mut |p| events.push(p)).unwrap();
    assert_eq!((r.added, r.updated, r.relinked, r.missing, r.unchanged), (4, 0, 0, 0, 0));
    assert!(r.errors.iter().any(|e| e.path.ends_with("broken.mp3")), "{:?}", r.errors);
    assert_eq!(events.last(), Some(&ScanProgress::Reading { done: 4, total: 4 }));
    let broken = env.lib.track_by_path(&root.join("b/broken.mp3")).unwrap().unwrap();
    assert_eq!(env.lib.track(broken).unwrap().unwrap().title, "broken");

    let r = env.lib.scan(&mut |_| {}).unwrap();
    assert_eq!((r.added, r.updated, r.unchanged), (0, 0, 4));

    // Content change.
    let one = env.lib.track_by_path(&root.join("a/one.wav")).unwrap().unwrap();
    write_wav(&root.join("a/one.wav"), 2.0, 220.0);
    // Move + rename; user data must follow the file.
    let two = env.lib.track_by_path(&root.join("a/two.wav")).unwrap().unwrap();
    env.lib.set_rating(two, 4).unwrap();
    std::fs::create_dir_all(root.join("c")).unwrap();
    std::fs::rename(root.join("a/two.wav"), root.join("c/two renamed.wav")).unwrap();
    // Deletion.
    let three = env.lib.track_by_path(&root.join("b/three.wav")).unwrap().unwrap();
    std::fs::remove_file(root.join("b/three.wav")).unwrap();

    let r = env.lib.scan(&mut |_| {}).unwrap();
    assert_eq!((r.added, r.updated, r.relinked, r.missing, r.unchanged), (0, 1, 1, 1, 1));
    assert!((env.lib.track(one).unwrap().unwrap().duration_secs - 2.0).abs() < 0.01);
    let moved = env.lib.track(two).unwrap().unwrap();
    assert_eq!(moved.path, root.join("c/two renamed.wav"));
    assert_eq!(moved.rating, 4);
    assert!(env.lib.track(three).unwrap().unwrap().missing);
    assert!(!env.lib.tracks_needing_analysis().unwrap().contains(&three));
    assert_eq!(env.lib.tracks().unwrap().len(), 4);

    // The file comes back.
    write_wav(&root.join("b/three.wav"), 1.4, 440.0);
    let r = env.lib.scan(&mut |_| {}).unwrap();
    assert_eq!((r.added, r.updated, r.missing), (0, 1, 0));
    assert!(!env.lib.track(three).unwrap().unwrap().missing);

    // Removing the root keeps the tracks.
    env.lib.remove_root(&root).unwrap();
    assert!(env.lib.roots().unwrap().is_empty());
    assert_eq!(env.lib.tracks().unwrap().len(), 4);
}

#[test]
fn analysis_round_trip_and_grid_ownership() {
    let mut env = Env::new();
    let path = env.music().join("t.wav");
    write_wav(&path, 2.0, 440.0);
    let id = env.lib.import_file(&path).unwrap();
    assert_eq!(env.lib.tracks_needing_analysis().unwrap(), [id]);
    assert_eq!(env.lib.analysis(id).unwrap(), None);

    // Stale analysis still needs re-analysis.
    env.lib.set_analysis(id, &analysis(120.0, ANALYZER_VERSION - 1)).unwrap();
    assert_eq!(env.lib.tracks_needing_analysis().unwrap(), [id]);

    let a = analysis(128.0, ANALYZER_VERSION);
    env.lib.set_analysis(id, &a).unwrap();
    assert_eq!(env.lib.analysis(id).unwrap(), Some(a.clone()));
    assert!(env.lib.tracks_needing_analysis().unwrap().is_empty());
    let row = env.lib.track(id).unwrap().unwrap();
    assert_eq!((row.bpm, row.key, row.analyzed, row.grid_locked), (Some(128.0), Some(Key::new(9, true)), true, false));
    assert_eq!(row.grid_confidence, Some(1.0));

    // A user edit survives re-analysis...
    let mut user = BeatGrid::new(BeatMap::constant(0.1, 127.5).unwrap(), GridSource::Manual);
    user.flags = GridFlags::SNAPPED;
    env.lib.set_grid(id, &user).unwrap();
    let a2 = analysis(126.0, ANALYZER_VERSION);
    env.lib.set_analysis(id, &a2).unwrap();
    assert_eq!(env.lib.grid(id).unwrap(), Some(user.clone()));
    let stored = env.lib.analysis(id).unwrap().unwrap();
    assert_eq!(stored.grid, Some(user));
    assert_eq!(stored.lufs, a2.lufs);
    let row = env.lib.track(id).unwrap().unwrap();
    assert_eq!((row.bpm, row.grid_flags()), (Some(127.5), GridFlags::SNAPPED));

    // ...until the user resets it to the analyzer's grid.
    env.lib.reset_grid(id).unwrap();
    assert_eq!(env.lib.grid(id).unwrap(), a2.grid);
    assert_eq!(env.lib.track(id).unwrap().unwrap().bpm, Some(126.0));

    // Locked grids are protected too, whatever their source.
    let mut locked = a2.grid.clone().unwrap();
    locked.locked = true;
    env.lib.set_grid(id, &locked).unwrap();
    env.lib.set_analysis(id, &analysis(90.0, ANALYZER_VERSION)).unwrap();
    assert_eq!(env.lib.grid(id).unwrap(), Some(locked));
    assert!(env.lib.track(id).unwrap().unwrap().grid_locked);

    // No grid from analysis removes an unprotected auto grid.
    env.lib.reset_grid(id).unwrap();
    let mut none = analysis(90.0, ANALYZER_VERSION);
    none.grid = None;
    env.lib.set_analysis(id, &none).unwrap();
    assert_eq!(env.lib.grid(id).unwrap(), None);
    assert_eq!(env.lib.track(id).unwrap().unwrap().bpm, None);

    assert!(matches!(env.lib.set_analysis(999, &a), Err(Error::NoTrack(999))));
    assert!(matches!(env.lib.set_grid(999, &a.grid.unwrap()), Err(Error::NoTrack(999))));
}

#[test]
fn import_files_and_folder_queries() {
    let mut env = Env::new();
    let m = env.music();
    write_wav(&m.join("a.wav"), 0.5, 440.0);
    write_wav(&m.join("b.wav"), 0.6, 440.0);
    write_wav(&m.join("sub/c.wav"), 0.7, 440.0);
    std::fs::write(m.join("notes.txt"), "not audio").unwrap();
    let files = vec![m.join("a.wav"), m.join("notes.txt"), m.join("sub/c.wav")];
    let r = env.lib.import_files(&files, &mut |_| {}).unwrap();
    assert_eq!((r.added, r.ids.len(), r.unchanged), (2, 2, 0));
    // Again: nothing to read, same ids.
    let again = env.lib.import_files(&files, &mut |_| {}).unwrap();
    assert_eq!((again.added, again.unchanged), (0, 2));
    assert_eq!(again.ids, r.ids);

    // The folder view: direct children, or everything below.
    let m = m.canonicalize().unwrap();
    let direct: Vec<String> = env
        .lib
        .tracks_under(&m, false)
        .unwrap()
        .iter()
        .map(|t| t.path.file_name().unwrap().to_string_lossy().into())
        .collect();
    assert_eq!(direct, ["a.wav"]);
    assert_eq!(env.lib.tracks_under(&m, true).unwrap().len(), 2);
    // A sibling folder whose name starts the same is not "under".
    write_wav(&m.join("sub2/d.wav"), 0.5, 440.0);
    env.lib.import_files(&[m.join("sub2/d.wav")], &mut |_| {}).unwrap();
    assert_eq!(env.lib.tracks_under(&m.join("sub"), true).unwrap().len(), 1);
}

#[test]
fn analysis_errors_are_not_retried() {
    let mut env = Env::new();
    write_wav(&env.music().join("bad.wav"), 0.5, 440.0);
    let id = env.lib.import_file(&env.music().join("bad.wav")).unwrap();
    assert_eq!(env.lib.tracks_needing_analysis().unwrap(), [id]);
    env.lib.set_analysis_error(id, "decoder exploded").unwrap();
    assert!(env.lib.tracks_needing_analysis().unwrap().is_empty());
    let row = env.lib.track(id).unwrap().unwrap();
    assert!(row.analysis_failed && !row.analyzed);
    assert_eq!(env.lib.analysis_error(id).unwrap().as_deref(), Some("decoder exploded"));
    // A successful analysis clears the failure.
    env.lib.set_analysis(id, &analysis(128.0, ANALYZER_VERSION)).unwrap();
    let row = env.lib.track(id).unwrap().unwrap();
    assert!(!row.analysis_failed && row.analyzed);
    assert_eq!(row.analysis_version, Some(ANALYZER_VERSION));
    assert_eq!(env.lib.analysis_error(id).unwrap(), None);
}

#[test]
fn cues_waveform_and_user_fields() {
    let mut env = Env::new();
    let path = env.music().join("t.wav");
    write_wav(&path, 1.0, 440.0);
    let id = env.lib.import_file(&path).unwrap();
    assert_eq!(env.lib.cues(id).unwrap(), TrackCues::default());

    let mut cues = TrackCues { main_cue_secs: 1.25, points: Vec::new() };
    cues.set_hotcue(CuePoint {
        slot: Some(2),
        kind: CueKind::Cue,
        start_secs: 3.5,
        len_secs: 0.0,
        name: "Drop".into(),
        color: 0xff0000,
    });
    cues.points.push(CuePoint {
        slot: None,
        kind: CueKind::Loop,
        start_secs: 10.0,
        len_secs: 1.875,
        name: String::new(),
        color: 0x00ff00,
    });
    env.lib.set_cues(id, &cues).unwrap();
    assert_eq!(env.lib.cues(id).unwrap(), cues);

    env.lib.set_waveform(id, &[1, 2, 3, 255]).unwrap();
    assert_eq!(env.lib.waveform(id).unwrap(), Some(vec![1, 2, 3, 255]));

    env.lib.set_rating(id, 9).unwrap();
    env.lib.set_color(id, Some(0x123456)).unwrap();
    let row = env.lib.track(id).unwrap().unwrap();
    assert_eq!((row.rating, row.color), (5, Some(0x123456)));
    env.lib.set_color(id, None).unwrap();
    assert_eq!(env.lib.track(id).unwrap().unwrap().color, None);

    assert!(matches!(env.lib.cues(42), Err(Error::NoTrack(42))));
    assert!(matches!(env.lib.set_rating(42, 1), Err(Error::NoTrack(42))));
    assert!(env.lib.set_waveform(42, &[]).is_err());

    env.lib.remove_tracks(&[id]).unwrap();
    assert_eq!(env.lib.track(id).unwrap(), None);
    assert_eq!(env.lib.waveform(id).unwrap(), None);
}

fn three_tracks(env: &mut Env) -> Vec<i64> {
    (0..3)
        .map(|i| {
            let p = env.music().join(format!("{i}.wav"));
            write_wav(&p, 0.2 + i as f32 * 0.1, 440.0);
            env.lib.import_file(&p).unwrap()
        })
        .collect()
}

#[test]
fn playlists() {
    let mut env = Env::new();
    let t = three_tracks(&mut env);
    let lib = &mut env.lib;

    let folder = lib.create_playlist(None, "Sets", true).unwrap();
    let sub = lib.create_playlist(Some(folder), "2026", true).unwrap();
    let a = lib.create_playlist(Some(folder), "Warmup", false).unwrap();
    let b = lib.create_playlist(Some(sub), "Peak", false).unwrap();
    let top = lib.create_playlist(None, "Crates", false).unwrap();
    assert!(matches!(lib.create_playlist(Some(a), "x", false), Err(Error::Invalid(_))));
    assert!(matches!(lib.create_playlist(Some(999), "x", false), Err(Error::NoPlaylist(999))));

    lib.add_to_playlist(a, &[t[0], t[1]], None).unwrap();
    lib.add_to_playlist(a, &[t[2]], Some(1)).unwrap();
    lib.add_to_playlist(a, &[t[0]], Some(100)).unwrap();
    assert_eq!(lib.playlist_tracks(a).unwrap(), [t[0], t[2], t[1], t[0]]);
    lib.move_in_playlist(a, 3, 0).unwrap();
    assert_eq!(lib.playlist_tracks(a).unwrap(), [t[0], t[0], t[2], t[1]]);
    lib.move_in_playlist(a, 0, 3).unwrap();
    assert_eq!(lib.playlist_tracks(a).unwrap(), [t[0], t[2], t[1], t[0]]);
    lib.remove_from_playlist(a, &[1, 3, 17]).unwrap();
    assert_eq!(lib.playlist_tracks(a).unwrap(), [t[0], t[1]]);
    assert!(lib.move_in_playlist(a, 5, 0).is_err());
    assert!(matches!(lib.add_to_playlist(folder, &[t[0]], None), Err(Error::Invalid(_))));
    assert!(matches!(lib.add_to_playlist(a, &[999], None), Err(Error::NoTrack(999))));
    lib.add_to_playlist(b, &[t[2]], None).unwrap();

    // Found by name and kind, else created; only missing tracks are added.
    assert_eq!(lib.playlist_named(Some(folder), "Warmup", false).unwrap(), a);
    assert_ne!(lib.playlist_named(Some(folder), "Warmup", true).unwrap(), a);
    assert_eq!(lib.add_missing_to_playlist(a, &[t[1], t[2], t[2]]).unwrap(), 1);
    assert_eq!(lib.playlist_tracks(a).unwrap(), [t[0], t[1], t[2]]);
    lib.remove_from_playlist(a, &[2]).unwrap();
    let extra = lib.playlist_named(Some(folder), "Warmup", true).unwrap();
    lib.delete_playlist(extra).unwrap();

    lib.rename_playlist(a, "Opening").unwrap();
    assert!(lib.rename_playlist(999, "x").is_err());

    let tree = lib.playlist_tree().unwrap();
    let names = |nodes: &[rille_library::PlaylistNode]| nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&tree), ["Sets", "Crates"]);
    assert_eq!(names(&tree[0].children), ["2026", "Opening"]);
    assert_eq!(tree[0].children[1].track_count, 2);
    assert_eq!(tree[0].children[0].children[0].name, "Peak");
    assert_eq!(tree[0].children[0].children[0].parent, Some(sub));

    // Moving a folder into its own subtree is refused.
    assert!(matches!(lib.move_playlist(folder, Some(sub)), Err(Error::Invalid(_))));
    lib.move_playlist(top, Some(sub)).unwrap();
    assert_eq!(names(&lib.playlist_tree().unwrap()[0].children[0].children), ["Peak", "Crates"]);

    // Deleting a track removes it from playlists; deleting a folder removes its subtree.
    lib.remove_tracks(&[t[1]]).unwrap();
    assert_eq!(lib.playlist_tracks(a).unwrap(), [t[0]]);
    lib.delete_playlist(folder).unwrap();
    assert!(lib.playlist_tree().unwrap().is_empty());
    assert!(matches!(lib.playlist_tracks(b), Err(Error::NoPlaylist(_))));
}

#[test]
fn history() {
    let mut env = Env::new();
    let t = three_tracks(&mut env);
    let lib = &mut env.lib;
    let s1 = lib.start_history_session().unwrap();
    lib.log_played(s1, t[1], 0).unwrap();
    lib.log_played(s1, t[0], 1).unwrap();
    lib.log_played(s1, t[1], 2).unwrap();
    let s2 = lib.start_history_session().unwrap();
    lib.log_played(s2, t[2], 3).unwrap();
    assert!(lib.log_played(999, t[2], 0).is_err());
    assert!(lib.log_played(s2, 999, 0).is_err());

    assert_eq!(lib.history_tracks(s1).unwrap(), [t[1], t[0], t[1]]);
    assert_eq!(lib.history_entries(s1).unwrap().iter().map(|e| e.deck).collect::<Vec<_>>(), [0, 1, 2]);
    let sessions = lib.history_sessions().unwrap();
    assert_eq!(sessions.iter().map(|s| (s.id, s.track_count)).collect::<Vec<_>>(), [(s2, 1), (s1, 3)]);

    let row = lib.track(t[1]).unwrap().unwrap();
    assert_eq!(row.play_count, 2);
    assert!(row.last_played.is_some());
    assert_eq!(lib.track(t[2]).unwrap().unwrap().play_count, 1);

    lib.delete_history_session(s1).unwrap();
    assert_eq!(lib.history_sessions().unwrap().len(), 1);
}

const NML: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<NML VERSION="19"><HEAD COMPANY="www.native-instruments.com" PROGRAM="Traktor"></HEAD>
<MUSICFOLDERS></MUSICFOLDERS>
<COLLECTION ENTRIES="3">
<ENTRY MODIFIED_DATE="2024/3/1" TITLE="Known" ARTIST="Someone" LOCK="1">
<LOCATION DIR="/:Volumes/:Music/:techno/:" FILE="known &amp; loved.wav" VOLUME="Music" VOLUMEID="Music"></LOCATION>
<ALBUM TITLE="Whatever"></ALBUM>
<INFO BITRATE="1411200" GENRE="Techno" LABEL="Label &quot;X&quot;" COMMENT="nml comment" PLAYCOUNT="7" RANKING="204" COLOR="4"></INFO>
<TEMPO BPM="128.000061" BPM_QUALITY="100.000000"></TEMPO>
<MUSICAL_KEY VALUE="21"></MUSICAL_KEY>
<CUE_V2 NAME="AutoGrid" DISPL_ORDER="0" TYPE="4" START="52.5" LEN="0" REPEATS="-1" HOTCUE="0">
<GRID BPM="128.000061"></GRID>
</CUE_V2>
<CUE_V2 NAME="n.n." DISPL_ORDER="0" TYPE="0" START="15052.5" LEN="0" REPEATS="-1" HOTCUE="1"></CUE_V2>
<CUE_V2 NAME="Build" DISPL_ORDER="0" TYPE="5" START="30052.5" LEN="7500" REPEATS="-1" HOTCUE="2" COLOR="#00FF00"></CUE_V2>
<CUE_V2 NAME="Load" DISPL_ORDER="0" TYPE="3" START="1000" LEN="0" REPEATS="-1" HOTCUE="-1"></CUE_V2>
</ENTRY>
<ENTRY TITLE="Gone"><LOCATION DIR="/:Volumes/:Music/:" FILE="gone.mp3" VOLUME="Music"></LOCATION>
<TEMPO BPM="125"></TEMPO></ENTRY>
<ENTRY TITLE="Elsewhere"><LOCATION DIR="/:Users/:me/:" FILE="x.mp3" VOLUME="Macintosh HD"></LOCATION></ENTRY>
</COLLECTION>
<PLAYLISTS><NODE TYPE="FOLDER" NAME="$ROOT"><SUBNODES COUNT="1"><NODE TYPE="PLAYLIST" NAME="P">
<PLAYLIST ENTRIES="1" TYPE="LIST"><ENTRY><PRIMARYKEY TYPE="TRACK" KEY="Music/:Volumes/:Music/:techno/:known &amp; loved.wav"></PRIMARYKEY></ENTRY></PLAYLIST>
</NODE></SUBNODES></NODE></PLAYLISTS>
</NML>
"##;

#[test]
fn nml_import() {
    let mut env = Env::new();
    let music = env.music();
    let file = music.join("techno/known & loved.wav");
    write_wav(&file, 1.0, 440.0);
    let nml = env.dir.path().join("collection.nml");
    std::fs::write(&nml, NML).unwrap();
    let map = |p: &str| p.strip_prefix("/Volumes/Music/").map(|rest| music.join(rest));

    let report = env.lib.import_nml(&nml, &map).unwrap();
    assert_eq!(report.entries, 3);
    assert_eq!((report.matched, report.imported, report.grids, report.cues), (1, 1, 1, 4));
    assert_eq!(report.missing, ["/Volumes/Music/gone.mp3", "/Users/me/x.mp3"]);
    assert!(report.errors.is_empty());

    let id = env.lib.track_by_path(&file).unwrap().unwrap();
    let row = env.lib.track(id).unwrap().unwrap();
    assert_eq!(row.rating, 4);
    assert_eq!(
        (row.genre.as_str(), row.label.as_str(), row.comment.as_str()),
        ("Techno", "Label \"X\"", "nml comment")
    );
    assert_eq!(row.play_count, 7);
    assert_eq!(row.key, Some(Key::new(9, true)));
    assert_eq!(row.color, Some(0x5fd068));
    assert!((row.bpm.unwrap() - 128.000061).abs() < 1e-9);
    assert!(row.grid_locked);

    let grid = env.lib.grid(id).unwrap().unwrap();
    assert_eq!(grid.source, GridSource::ImportedNml);
    assert_eq!(grid.map, BeatMap::constant(0.0525, 128.000061).unwrap());

    let cues = env.lib.cues(id).unwrap();
    assert_eq!(cues.main_cue_secs, 1.0);
    let hot1 = cues.hotcue(1).unwrap();
    assert_eq!((hot1.kind, hot1.name.as_str(), hot1.start_secs), (CueKind::Cue, "", 15.0525));
    let hot2 = cues.hotcue(2).unwrap();
    assert_eq!((hot2.kind, hot2.len_secs, hot2.color, hot2.name.as_str()), (CueKind::Loop, 7.5, 0x00ff00, "Build"));
    assert_eq!(cues.hotcue(0).unwrap().kind, CueKind::Grid);

    // Imported grids survive re-analysis, even when not locked.
    let mut unlocked = grid.clone();
    unlocked.locked = false;
    // Re-import without LOCK to get an unlocked imported grid.
    std::fs::write(&nml, NML.replace(r#" LOCK="1""#, "")).unwrap();
    let report = env.lib.import_nml(&nml, &map).unwrap();
    assert_eq!((report.matched, report.imported, report.grids), (1, 0, 1));
    assert_eq!(env.lib.grid(id).unwrap(), Some(unlocked.clone()));
    env.lib.set_analysis(id, &analysis(130.0, ANALYZER_VERSION)).unwrap();
    assert_eq!(env.lib.grid(id).unwrap(), Some(unlocked));

    // A grid edited here is not replaced by a later NML import.
    let user = BeatGrid::new(BeatMap::constant(0.06, 128.0).unwrap(), GridSource::Manual);
    env.lib.set_grid(id, &user).unwrap();
    let report = env.lib.import_nml(&nml, &map).unwrap();
    assert_eq!(report.grids, 0);
    assert_eq!(env.lib.grid(id).unwrap(), Some(user));

    assert!(matches!(env.lib.import_nml(&env.dir.path().join("none.nml"), &map), Err(Error::Io { .. })));
}

/// Imports ~/Music/320 into a temp database: `cargo test -p rille-library -- --ignored --nocapture`.
#[test]
#[ignore]
fn real_music_folder() {
    let root = PathBuf::from(std::env::var("HOME").unwrap()).join("Music/320");
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open(&dir.path().join("lib.sqlite"), &dir.path().join("cache")).unwrap();
    lib.add_root(&root).unwrap();
    let t = std::time::Instant::now();
    let report = lib.scan(&mut |_| {}).unwrap();
    println!("scan took {:?}: {report:#?}", t.elapsed());
    let rows = lib.tracks().unwrap();
    println!("{} tracks, {} with cover", rows.len(), rows.iter().filter(|r| r.has_cover).count());
    for r in rows.iter().take(8) {
        println!(
            "{:>3} {} - {} [{}] {} | {} {:.1}s {:?}kbps {:?}Hz bpm {:?} key {:?}",
            r.id,
            r.artist,
            r.title,
            r.remixer,
            r.album,
            r.label,
            r.duration_secs,
            r.bitrate,
            r.sample_rate,
            r.bpm,
            r.key.map(|k| k.camelot())
        );
    }
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| !r.title.is_empty() && r.duration_secs > 0.0));
    let rescan = lib.scan(&mut |_| {}).unwrap();
    assert_eq!(rescan.unchanged, rows.len());
}

/// Imports the FLACs in ~/Downloads one by one (drag & drop path).
#[test]
#[ignore]
fn real_flac_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open_in_memory(&dir.path().join("cache")).unwrap();
    let downloads = PathBuf::from(std::env::var("HOME").unwrap()).join("Downloads");
    for entry in std::fs::read_dir(downloads).unwrap().flatten() {
        if entry.path().extension().is_some_and(|e| e == "flac") {
            let id = lib.import_file(&entry.path()).unwrap();
            let r = lib.track(id).unwrap().unwrap();
            println!(
                "{} - {} | {} | {:.1}s {:?}Hz cover {} key {:?}",
                r.artist,
                r.title,
                r.label,
                r.duration_secs,
                r.sample_rate,
                r.has_cover,
                r.key.map(|k| k.camelot())
            );
            assert!(r.duration_secs > 0.0);
        }
    }
}

/// Scan throughput on 10k tiny generated files.
#[test]
#[ignore]
fn scan_ten_thousand_files() {
    let mut env = Env::new();
    for i in 0..10_000 {
        write_wav(&env.music().join(format!("{}/{i}.wav", i / 100)), 0.01 + i as f32 * 1e-6, 440.0);
    }
    env.lib.add_root(&env.music()).unwrap();
    let t = std::time::Instant::now();
    let r = env.lib.scan(&mut |_| {}).unwrap();
    println!("first scan {:?}", t.elapsed());
    assert_eq!(r.added, 10_000);
    let t = std::time::Instant::now();
    let r = env.lib.scan(&mut |_| {}).unwrap();
    println!("rescan {:?}", t.elapsed());
    assert_eq!(r.unchanged, 10_000);
    let t = std::time::Instant::now();
    assert_eq!(env.lib.tracks().unwrap().len(), 10_000);
    println!("tracks() {:?}", t.elapsed());
}
