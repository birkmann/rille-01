//! End-to-end: library import, loading with analysis, playing, sync, cue
//! persistence and grid edits, without audio hardware.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rille_analysis::synth::{self, Spec};
use rille_app::{App, BeatportList, GridEdit, Paths, SortKey, Source, StartOptions};
use rille_core::{BeatClock, Control, ControlEvent, ControlTarget, ControlValue};

fn write_wav(path: &Path, audio: &rille_decode::DecodedAudio) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    let n = audio.frames.len() as u32;
    let (sr, ch) = (audio.sample_rate, 2u16);
    let data = n * 4;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&ch.to_le_bytes()).unwrap();
    f.write_all(&sr.to_le_bytes()).unwrap();
    f.write_all(&(sr * 4).to_le_bytes()).unwrap();
    f.write_all(&4u16.to_le_bytes()).unwrap();
    f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data.to_le_bytes()).unwrap();
    for [l, r] in &audio.frames {
        f.write_all(&((l.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).unwrap();
        f.write_all(&((r.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).unwrap();
    }
}

fn wait_for(what: &str, secs: f64, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed().as_secs_f64() < secs, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn press(app: &App, deck: u8, c: Control) {
    for p in [true, false] {
        app.control(ControlEvent { target: ControlTarget::deck(deck, c), value: ControlValue::Press(p) });
    }
}

fn start(root: &Path) -> Arc<App> {
    App::start(StartOptions { paths: Paths::under(root), audio: false, midi: false, bundled_mappings: None }).unwrap()
}

#[test]
fn load_analyze_play_sync_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    for (name, bpm, seed) in [("a.wav", 124.0, 1), ("b.wav", 128.0, 2)] {
        let r = synth::render(&Spec { sections: vec![(48, bpm)], seed, ..Spec::default() });
        write_wav(&music.join(name), &r.audio);
    }
    let app = start(dir.path());
    let mut s = app.settings();
    s.background_analysis = false;
    app.set_settings(s);
    app.add_music_folder(music.clone());
    wait_for("scan", 20.0, || app.tracks(Source::Collection, "", SortKey::Title, false).len() == 2);
    let rows = app.tracks(Source::Collection, "", SortKey::Title, false);
    assert_eq!(rows[0].title, "a");
    assert_eq!(app.tracks(Source::Collection, "b", SortKey::Title, false).len(), 1);

    app.load_track(0, rows[0].id);
    app.load_track(1, rows[1].id);
    wait_for("analysis", 60.0, || app.deck(0).grid.is_some() && app.deck(1).grid.is_some());
    let g0 = app.deck(0).grid.unwrap();
    assert!((g0.bpm_at(0.0) - 124.0).abs() < 0.01, "{}", g0.bpm_at(0.0));
    assert!(app.deck(0).waveform.is_some());

    // Play A, sync B to it.
    press(&app, 0, Control::Play);
    press(&app, 1, Control::Sync);
    press(&app, 1, Control::Play);
    std::thread::sleep(Duration::from_millis(1500));
    app.tick();
    let snap = app.snapshot();
    assert!(snap.decks[0].playing && snap.decks[1].playing);
    assert!((snap.decks[1].bpm - 124.0).abs() < 0.1, "B follows at {}", snap.decks[1].bpm);
    assert!(snap.decks[1].phase_error.abs() < 0.01, "phase error {}", snap.decks[1].phase_error);

    // Set a hotcue; it is saved to the library.
    press(&app, 0, Control::Hotcue(1));
    std::thread::sleep(Duration::from_millis(100));
    app.tick();
    // Grid edit: move by 10 ms and check it is used and saved.
    let before = app.deck(1).grid.unwrap().secs_at(0.0);
    app.grid_edit(1, GridEdit::Move(10.0));
    let after = app.deck(1).grid.unwrap().secs_at(0.0);
    assert!((after - before - 0.010).abs() < 1e-9);
    app.shutdown();
    drop(app);

    // Reopen: analysis, cue and edited grid persisted.
    let app = start(dir.path());
    let rows = app.tracks(Source::Collection, "", SortKey::Title, false);
    assert!(rows.iter().all(|r| r.analyzed));
    assert!((rows[0].bpm.unwrap() - 124.0).abs() < 0.01);
    app.load_track(0, rows[0].id);
    app.load_track(1, rows[1].id);
    wait_for("reload", 20.0, || app.deck(0).grid.is_some() && app.deck(1).grid.is_some());
    std::thread::sleep(Duration::from_millis(200));
    let snap = app.snapshot();
    assert!(snap.decks[0].hotcues[0].is_some(), "hotcue persisted");
    assert!((app.deck(1).grid.unwrap().secs_at(0.0) - after).abs() < 1e-9, "edited grid persisted");
    app.shutdown();
}

/// Top-level playlist nodes: name, is folder, entries, children with entries.
type Shape = Vec<(String, bool, usize, Vec<(String, usize)>)>;

#[test]
fn streamed_tracks_live_apart_from_the_collection() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    paths.create().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    std::fs::create_dir_all(paths.beatport_cache()).unwrap();
    let r = synth::render(&Spec { sections: vec![(32, 126.0)], seed: 3, ..Spec::default() });
    write_wav(&music.join("local.wav"), &r.audio);
    // Tracks streamed in an earlier session, still in the cache; 43 was
    // downloaded for offline use.
    let cached = paths.beatport_cache().join("42.wav");
    write_wav(&cached, &r.audio);
    let offline = paths.beatport_cache().join("43.wav");
    write_wav(&offline, &r.audio);
    let other = paths.beatport_cache().join("44.wav");
    write_wav(&other, &r.audio);
    {
        let mut lib = rille_library::Library::open(&paths.library_db(), &paths.cache).unwrap();
        for (bp, file, title) in [(42, &cached, "Streamed"), (43, &offline, "Offline"), (44, &other, "Other")] {
            let tags = rille_library::Tags { title: title.into(), artist: "Someone".into(), ..Default::default() };
            let id = lib.upsert_streamed(bp, file, &tags, None).unwrap();
            lib.set_streamed_file(id, file).unwrap();
            if bp == 43 {
                lib.set_streamed_offline(&[id], true).unwrap();
            }
        }
    }

    let app = start(dir.path());
    let mut s = app.settings();
    s.background_analysis = false;
    app.set_settings(s);
    app.import_paths(vec![music.join("local.wav")], false);
    wait_for("import", 20.0, || app.track_count() == 1);
    assert_eq!((app.streamed_count(), app.offline_count()), (3, 1));
    let collection = app.tracks(Source::Collection, "", SortKey::Artist, false);
    assert_eq!(collection.len(), 1);
    assert_eq!(collection[0].beatport_id, None);
    let recent = app.tracks(Source::BeatportRecent, "", SortKey::Title, false);
    assert_eq!(recent.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Offline", "Other", "Streamed"]);
    let kept = app.tracks(Source::BeatportOffline, "", SortKey::Artist, false);
    assert_eq!((kept.len(), kept[0].beatport_id), (1, Some(43)));
    let recent: Vec<_> = recent.into_iter().filter(|r| r.beatport_id == Some(42)).collect();

    // Cached: it loads without asking Beatport.
    app.load_track(0, recent[0].id);
    wait_for("load", 20.0, || app.deck(0).audio.is_some() && !app.deck(0).loading);
    let deck = app.deck(0);
    assert_eq!((deck.title.as_str(), deck.download, deck.error), ("Streamed", None, None));
    assert_eq!(deck.path.as_deref(), Some(cached.as_path()));

    // Signed out: an empty search lists nothing, a search reports why.
    assert_eq!(app.beatport_account(), None);
    let empty = BeatportList::Search(String::new());
    app.beatport_open(empty.clone(), false);
    assert!(app.beatport_rows(&empty).is_empty());
    assert!(!app.beatport_status(&empty).loading);
    let search = BeatportList::Search("solomun".into());
    app.beatport_open(search.clone(), false);
    wait_for("search", 10.0, || !app.beatport_status(&search).loading);
    assert!(app.beatport_status(&search).error.is_some_and(|e| e.contains("not signed in")));

    // Clearing the streamed files keeps the offline download and the
    // file on the deck; the cleared track stays in the collection.
    let usage = app.beatport_cache_usage();
    assert_eq!((usage.files, usage.offline_files), (3, 1));
    app.clear_beatport_cache(false);
    assert!(cached.exists() && offline.exists() && !other.exists());
    let other_row = app.tracks(Source::BeatportRecent, "Other", SortKey::Title, false);
    assert!(other_row[0].missing, "out of the cache, downloads again when loaded");
    // Removing the download lets the offline track go too.
    app.beatport_remove_downloads(&[kept[0].id]);
    assert!(!offline.exists());
    assert_eq!(app.offline_count(), 0);
    assert!(app.tracks(Source::BeatportOffline, "", SortKey::Artist, false).is_empty());
    // Signed out, nothing is queued.
    app.beatport_download(vec![99]);
    assert!(!app.beatport_downloads().running);
    app.shutdown();
}

#[test]
fn import_folder_as_playlists() {
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("Set");
    std::fs::create_dir_all(set.join("Warmup")).unwrap();
    std::fs::create_dir_all(set.join("Empty")).unwrap();
    let r = synth::render(&Spec { sections: vec![(8, 125.0)], ..Spec::default() });
    for f in ["a.wav", "b.wav", "Warmup/c.wav"] {
        write_wav(&set.join(f), &r.audio);
    }
    let app = start(dir.path());
    let mut s = app.settings();
    s.background_analysis = false;
    app.set_settings(s);

    let shape = |app: &App| -> Shape {
        app.playlists()
            .iter()
            .map(|n| {
                let kids = n.children.iter().map(|c| (c.name.clone(), c.track_count)).collect();
                (n.name.clone(), n.is_folder, n.track_count, kids)
            })
            .collect()
    };
    let want = vec![("Set".to_string(), true, 0, vec![("Set".to_string(), 2), ("Warmup".to_string(), 1)])];

    app.import_folder_as_playlist(&set, true, false);
    wait_for("import", 20.0, || shape(&app) == want);
    assert_eq!(app.tracks(Source::Collection, "", SortKey::Title, false).len(), 3);

    // Again: the playlists are reused and nothing is doubled.
    app.import_folder_as_playlist(&set, true, false);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(shape(&app), want);

    // Without subfolders: one playlist named after the folder.
    app.import_folder_as_playlist(&set.join("Warmup"), false, false);
    wait_for("playlist", 20.0, || shape(&app).len() == 2);
    assert_eq!(shape(&app)[1], ("Warmup".to_string(), false, 1, vec![]));
    app.shutdown();
}

/// One bar of clicks at 128 BPM: a loop file without a beatgrid.
fn click_loop() -> rille_decode::DecodedAudio {
    let sr = 44_100;
    let n = (1.875 * f64::from(sr)) as usize;
    let mut frames = vec![[0.0f32; 2]; n];
    for b in 0..4 {
        let at = (b as f64 * 60.0 / 128.0 * f64::from(sr)) as usize;
        frames.iter_mut().skip(at).take(200).for_each(|f| *f = [0.8, 0.8]);
    }
    rille_decode::DecodedAudio { sample_rate: sr, frames }
}

#[test]
fn remix_deck_load_trigger_capture_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    write_wav(&music.join("loop.wav"), &click_loop());
    let r = synth::render(&Spec { sections: vec![(64, 124.0)], seed: 3, ..Spec::default() });
    write_wav(&music.join("track.wav"), &r.audio);

    let app = start(dir.path());
    let mut s = app.settings();
    s.background_analysis = false;
    s.remix_decks = "C".into();
    app.set_settings(s);
    assert!(app.is_remix_deck(2) && !app.is_remix_deck(0));
    app.add_music_folder(music.clone());
    wait_for("scan", 20.0, || app.tracks(Source::Collection, "", SortKey::Title, false).len() == 2);
    let rows = app.tracks(Source::Collection, "", SortKey::Title, false);
    let (looped, track) = (rows[0].id, rows[1].id);
    assert_eq!(rows[0].title, "loop");

    // A loop file goes into the first free cell; its length gives 128 BPM,
    // and the first sample sets the deck's tempo.
    app.load_track(2, looped);
    let cell0 = || app.deck(2).remix.and_then(|s| s.cells[0].clone());
    wait_for("cell load", 20.0, || cell0().is_some_and(|c| c.audio.is_some()));
    let c = cell0().unwrap();
    assert!(c.looped && (c.bpm - 128.0).abs() < 0.01, "{c:?}");
    assert_eq!(app.deck(2).title, "Remix Deck C");
    wait_for("remix state", 5.0, || app.snapshot().remix[2].cells[0].loaded);
    assert!((app.snapshot().decks[2].track_bpm - 128.0).abs() < 0.01);

    // Pad 1 starts the stopped deck with it.
    press(&app, 2, Control::RemixPad(1));
    wait_for("trigger", 5.0, || app.snapshot().remix[2].slots[0].cell == Some(0));
    assert!(app.snapshot().decks[2].playing);

    // Capture a loop from deck A (its source by default) into pad 2's cell
    // (slot 2, row 1): its loop size, 4 beats, at the track's tempo.
    app.load_track(0, track);
    wait_for("analysis", 60.0, || app.deck(0).grid.is_some() && app.snapshot().decks[0].loaded);
    press(&app, 0, Control::Play);
    std::thread::sleep(Duration::from_millis(300));
    press(&app, 2, Control::RemixPadCapture(2));
    let cell16 = app.deck(2).remix.and_then(|s| s.cells[16].clone()).expect("captured");
    assert!(cell16.looped && (cell16.bpm - 124.0).abs() < 0.05, "{cell16:?}");
    assert!((cell16.len_secs - 4.0 * 60.0 / 124.0).abs() < 0.01);
    assert!(cell16.name.ends_with("1 bar"), "{}", cell16.name);
    // TYPE + pad 1: one-shot.
    press(&app, 2, Control::RemixPadType(1));
    assert!(!cell0().unwrap().looped);
    app.shutdown();
    drop(app);

    // The set comes back, decoded, on the next start.
    let app = start(dir.path());
    assert!(app.is_remix_deck(2));
    let cells = || app.deck(2).remix.unwrap().cells.clone();
    wait_for("set reload", 20.0, || cells().iter().flatten().filter(|c| c.audio.is_some()).count() == 2);
    assert!(!cells()[0].as_ref().unwrap().looped);
    assert_eq!(cells()[16].as_ref().unwrap(), &cell16);
    wait_for("engine cells", 5.0, || app.snapshot().remix[2].cells[16].loaded);

    // Back to a track deck.
    let mut s = app.settings();
    s.remix_decks.clear();
    app.set_settings(s);
    assert!(!app.is_remix_deck(2));
    wait_for("track deck", 5.0, || !app.snapshot().decks[2].remix);
    app.shutdown();
}

/// Against the real Beatport API with a signed-in account (downloads about
/// 25 MB): `RILLE_BEATPORT_TOKEN=~/.local/share/rille/beatport-token.json
/// cargo test -p rille-app --test app beatport_live -- --ignored`.
#[test]
#[ignore]
fn beatport_live_download_queue_and_covers() {
    let Some(token) = std::env::var_os("RILLE_BEATPORT_TOKEN") else { return };
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    paths.create().unwrap();
    std::fs::copy(token, paths.beatport_token()).unwrap();
    let app = start(dir.path());
    let mut s = app.settings();
    s.background_analysis = false;
    s.beatport_quality = "high".into();
    app.set_settings(s);
    assert!(app.beatport_account().is_some(), "signed in");

    // Two tracks for offline use, from their ids alone.
    app.beatport_download(vec![21298604, 19374165]);
    wait_for("download start", 10.0, || app.beatport_downloads().running);
    wait_for("downloads", 240.0, || !app.beatport_downloads().running);
    let q = app.beatport_downloads();
    assert_eq!((q.done, q.failed), (2, 0));
    let kept = app.tracks(Source::BeatportOffline, "", SortKey::Artist, false);
    assert_eq!(kept.len(), 2);
    for r in &kept {
        assert!(r.path.exists() && r.beatport_offline && !r.missing, "{:?}", r.path);
        assert!(r.has_cover, "catalog cover for {}", r.title);
    }

    // A deck shows the cover while the track still downloads.
    app.load_beatport(0, 20683478);
    wait_for("cover", 30.0, || app.deck(0).cover.is_some());
    wait_for("load", 240.0, || app.deck(0).audio.is_some() && !app.deck(0).loading);
    assert!(app.deck(0).error.is_none());
    app.shutdown();
}

#[test]
fn suggestions_follow_the_loaded_track() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let am = rille_core::Key::new(9, true);
    // a: the reference; b: same tempo and key; c: a bit faster; d: too fast to mix.
    let tracks = [("a.wav", 124.0, am, 1), ("b.wav", 124.0, am, 2), ("c.wav", 126.0, am, 3), ("d.wav", 150.0, am, 4)];
    for (name, bpm, key, seed) in tracks {
        let r = synth::render(&Spec { sections: vec![(32, bpm)], key, seed, ..Spec::default() });
        write_wav(&music.join(name), &r.audio);
    }
    let app = start(dir.path());
    app.add_music_folder(music);
    let analyzed = || {
        let rows = app.tracks(Source::Collection, "", SortKey::Title, false);
        rows.len() == 4 && rows.iter().all(|r| r.bpm.is_some())
    };
    wait_for("scan and analysis", 120.0, analyzed);
    let rows = app.tracks(Source::Collection, "", SortKey::Title, false);
    let id = |title: &str| rows.iter().find(|r| r.title == title).unwrap().id;
    let suggested =
        || app.tracks(Source::Suggestions, "", SortKey::Artist, false).iter().map(|r| r.id).collect::<Vec<_>>();

    // Off by default.
    app.load_track(0, id("a"));
    app.tick();
    assert!(app.suggestion_reference().is_none());
    assert!(suggested().is_empty());

    let mut s = app.settings();
    s.suggestions = true;
    app.set_settings(s);
    app.tick();
    assert_eq!(app.suggestion_reference().map(|(deck, r)| (deck, r.id)), Some((0, id("a"))));
    let ids = suggested();
    assert_eq!(ids.first(), Some(&id("b")), "{ids:?}");
    assert!(ids.contains(&id("c")) && !ids.contains(&id("d")) && !ids.contains(&id("a")), "{ids:?}");

    // A track loaded on another deck is left out.
    app.load_track(1, id("b"));
    app.tick();
    assert!(!suggested().contains(&id("b")));

    let mut s = app.settings();
    s.suggestions = false;
    app.set_settings(s);
    assert!(suggested().is_empty());
    app.shutdown();
}

#[test]
fn pause_and_cancel_stick() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(dir.path());
    assert!(app.settings().background_analysis);

    // A pause is kept over a restart.
    app.set_analysis_paused(true);
    app.shutdown();
    drop(app);
    let app = start(dir.path());
    assert!(app.settings().analysis_paused && app.analysis_progress().paused);
    app.set_analysis_paused(false);
    assert!(!app.analysis_progress().paused);

    // Cancel turns the background pass off, also after a restart.
    app.cancel_analysis();
    assert!(!app.settings().background_analysis);
    app.shutdown();
    drop(app);
    let app = start(dir.path());
    assert!(!app.settings().background_analysis && !app.analysis_progress().paused);
    app.shutdown();
}

#[test]
fn records_the_main_mix() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let r = synth::render(&Spec { sections: vec![(16, 126.0)], seed: 5, ..Spec::default() });
    write_wav(&music.join("a.wav"), &r.audio);
    let app = start(dir.path());
    app.add_music_folder(music);
    wait_for("scan", 20.0, || app.tracks(Source::Collection, "", SortKey::Title, false).len() == 1);
    let row = app.tracks(Source::Collection, "", SortKey::Title, false).remove(0);
    app.load_track(0, row.id);
    wait_for("load", 30.0, || app.deck(0).audio.is_some());

    let rec = ControlEvent { target: ControlTarget::global(Control::Record), value: ControlValue::Press(true) };
    app.control(rec);
    let (path, _) = app.recording().expect("recording");
    assert!(path.starts_with(dir.path().join("recordings")), "{}", path.display());
    press(&app, 0, Control::Play);
    wait_for("a second of audio", 10.0, || app.recording().is_some_and(|(_, secs)| secs > 1.0));
    let done = app.stop_recording().expect("a recording");
    assert!(app.recording().is_none());
    assert_eq!(done.dropped, 0);
    let mut wav = hound::WavReader::open(&done.path).unwrap();
    let frames = wav.duration() as f64 / f64::from(wav.spec().sample_rate);
    assert!((frames - done.secs).abs() < 0.01 && frames > 1.0, "{frames} vs {}", done.secs);
    assert!(wav.samples::<i32>().map(Result::unwrap).any(|s| s.abs() > 1 << 16), "the track is in it");

    // External mixing has no main mix to record.
    let mut s = app.settings();
    s.mixing = rille_app::settings::MixingMode::External;
    app.set_settings(s);
    assert!(app.start_recording().is_err());
    app.shutdown();
}

#[test]
fn cached_stems_come_with_the_track() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let r = synth::render(&Spec { sections: vec![(16, 126.0)], seed: 7, ..Spec::default() });
    write_wav(&music.join("a.wav"), &r.audio);
    let app = start(dir.path());
    app.add_music_folder(music);
    wait_for("scan", 20.0, || app.tracks(Source::Collection, "", SortKey::Title, false).len() == 1);
    let row = app.tracks(Source::Collection, "", SortKey::Title, false).remove(0);
    app.load_track(0, row.id);
    wait_for("load", 30.0, || app.deck(0).audio.is_some() && !app.deck(0).loading);
    let audio = app.deck(0).audio.unwrap();

    // No model and nothing cached: nothing to load.
    app.prepare_stems(0);
    assert_eq!(app.deck(0).stems, rille_app::stems::DeckStems::None);

    // Stems separated earlier (here: silence) are in the cache.
    let spec = hound::WavSpec {
        channels: 6,
        sample_rate: audio.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let path = Paths::under(dir.path()).cache.join("stems").join(format!("{}.wav", row.id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    for _ in 0..audio.frames.len() * 6 {
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
    app.prepare_stems(0);
    assert_eq!(app.deck(0).stems, rille_app::stems::DeckStems::Ready);
    wait_for("stems in the engine", 5.0, || app.snapshot().decks[0].stems);
    let mute = ControlEvent { target: ControlTarget::deck(0, Control::StemMute(4)), value: ControlValue::Press(true) };
    app.control(mute);
    wait_for("vocals muted", 5.0, || app.snapshot().decks[0].stem_mute == [false, false, false, true]);

    // Loading the track again brings them along.
    app.load_track(0, row.id);
    wait_for("reload with stems", 30.0, || app.snapshot().decks[0].stems && !app.deck(0).loading);
    assert_eq!(app.snapshot().decks[0].stem_mute, [false; 4], "a new load starts with every stem on");
    app.shutdown();
}

/// With `RILLE_STEM_MODEL=<htdemucs.onnx>`: a track is separated in the
/// background and its stems reach the deck and the cache.
#[test]
#[ignore]
fn separates_stems_with_the_model() {
    let Some(model) = std::env::var_os("RILLE_STEM_MODEL") else { return };
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    std::fs::create_dir_all(paths.data.join("models")).unwrap();
    std::os::unix::fs::symlink(&model, paths.data.join("models").join("htdemucs.onnx")).unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let r = synth::render(&Spec { sections: vec![(16, 126.0)], seed: 9, ..Spec::default() });
    write_wav(&music.join("a.wav"), &r.audio);
    let app = start(dir.path());
    assert_eq!(app.stem_model(), rille_app::stems::ModelState::Ready);
    app.add_music_folder(music);
    wait_for("scan", 20.0, || app.tracks(Source::Collection, "", SortKey::Title, false).len() == 1);
    let row = app.tracks(Source::Collection, "", SortKey::Title, false).remove(0);
    app.load_track(0, row.id);
    wait_for("load", 30.0, || app.deck(0).audio.is_some() && !app.deck(0).loading);
    let t = Instant::now();
    app.prepare_stems(0);
    let mut seen = Vec::new();
    wait_for("separation", 600.0, || {
        let s = app.deck(0).stems;
        if seen.last() != Some(&std::mem::discriminant(&s)) {
            seen.push(std::mem::discriminant(&s));
            eprintln!("{:?} after {:?}", s, t.elapsed());
        }
        matches!(s, rille_app::stems::DeckStems::Ready | rille_app::stems::DeckStems::Failed(_))
    });
    assert_eq!(app.deck(0).stems, rille_app::stems::DeckStems::Ready);
    wait_for("stems in the engine", 5.0, || app.snapshot().decks[0].stems);
    assert!(paths.cache.join("stems").join(format!("{}.wav", row.id)).exists());
    app.shutdown();
}

#[test]
fn load_lock_keeps_tracks_off_decks_on_air() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    for (name, seed) in [("a.wav", 1), ("b.wav", 2)] {
        let r = synth::render(&Spec { sections: vec![(16, 126.0)], seed, ..Spec::default() });
        write_wav(&music.join(name), &r.audio);
    }
    let app = start(dir.path());
    app.add_music_folder(music);
    wait_for("scan", 20.0, || app.tracks(Source::Collection, "", SortKey::Title, false).len() == 2);
    let rows = app.tracks(Source::Collection, "", SortKey::Title, false);
    let (a, b) = (rows[0].id, rows[1].id);
    app.load_track(0, a);
    wait_for("load", 30.0, || app.snapshot().decks[0].loaded);
    press(&app, 0, Control::Play);
    wait_for("play", 5.0, || app.snapshot().decks[0].playing);
    let set = |c: Control, deck: Option<u8>, v: f32| {
        let target = deck.map_or(ControlTarget::global(c), |d| ControlTarget::deck(d, c));
        app.control(ControlEvent { target, value: ControlValue::Absolute(v) });
    };

    // Off by default: a playing deck with its fader up still loads.
    assert!(!app.load_locked(0));
    let mut s = app.settings();
    s.load_lock = true;
    s.load_lock_level = 0.5;
    app.set_settings(s);
    assert!(app.load_locked(0), "playing, fader up");
    app.load_track(0, b);
    assert_eq!(app.deck(0).track_id, Some(a), "the load is refused");

    // Below the threshold, or cut by the crossfader, the deck is off air.
    set(Control::Volume, Some(0), 0.3);
    wait_for("fader down", 5.0, || !app.load_locked(0));
    set(Control::Volume, Some(0), 1.0);
    wait_for("fader up", 5.0, || app.load_locked(0));
    set(Control::Crossfader, None, 1.0);
    wait_for("crossfader to B", 5.0, || !app.load_locked(0));
    set(Control::Crossfader, None, 0.5);
    wait_for("crossfader centre", 5.0, || app.load_locked(0));

    // Decks left out of the list always load.
    let mut s = app.settings();
    s.load_lock_decks = "B".into();
    app.set_settings(s);
    assert!(!app.load_locked(0));
    app.load_track(0, b);
    assert_eq!(app.deck(0).track_id, Some(b));
    app.shutdown();
}
