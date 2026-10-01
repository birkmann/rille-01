//! The browser's track table: collection, playlists, history and explorer
//! folders, with multi-selection and actions on the selected rows.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(QString, search)]
        /// 0 collection, 1 playlist, 2 history session, 3 explorer folder,
        /// 6 suggestions, 7 Beatport search, 8 streamed tracks, 9 Beatport
        /// playlist, 10 Beatport chart (the browser tree's kinds).
        #[qproperty(i32, source_kind, cxx_name = "sourceKind")]
        /// Playlist or session id, the folder's path token, or the Beatport
        /// playlist id.
        #[qproperty(i64, source_id, cxx_name = "sourceId")]
        #[qproperty(QString, sort_key, cxx_name = "sortKey")]
        #[qproperty(bool, descending)]
        #[qproperty(i32, count)]
        #[qproperty(i32, selected_count, cxx_name = "selectedCount")]
        /// Rows of the current folder that are not in the collection yet.
        #[qproperty(i32, new_count, cxx_name = "newCount")]
        #[qproperty(QString, summary)]
        type TrackListModel = super::TrackListRust;

        #[qinvokable]
        fn refresh(self: Pin<&mut TrackListModel>);

        /// Re-reads the rows whose tracks changed (analysis, rating, color).
        #[qinvokable]
        #[cxx_name = "updateChanged"]
        fn update_changed(self: Pin<&mut TrackListModel>);

        #[qinvokable]
        #[cxx_name = "trackId"]
        fn track_id(self: &TrackListModel, row: i32) -> i64;

        #[qinvokable]
        #[cxx_name = "sortBy"]
        fn sort_by(self: Pin<&mut TrackListModel>, key: &QString);

        /// Click selection: `mode` 0 = only this row, 1 = toggle (Ctrl),
        /// 2 = range from the last clicked row (Shift).
        #[qinvokable]
        fn select(self: Pin<&mut TrackListModel>, row: i32, mode: i32);

        #[qinvokable]
        #[cxx_name = "selectAll"]
        fn select_all(self: Pin<&mut TrackListModel>);

        #[qinvokable]
        #[cxx_name = "isSelected"]
        fn is_selected(self: &TrackListModel, row: i32) -> bool;

        /// Loads a row on a deck (importing a file from the explorer).
        #[qinvokable]
        #[cxx_name = "loadRow"]
        fn load_row(self: &TrackListModel, row: i32, deck: i32);

        /// Loads a row into cell `cell` (0..64) of remix deck `deck`.
        #[qinvokable]
        #[cxx_name = "loadRowToCell"]
        fn load_row_to_cell(self: &TrackListModel, row: i32, deck: i32, cell: i32);

        /// Path token of a row, for drag and drop.
        #[qinvokable]
        #[cxx_name = "pathToken"]
        fn path_token(self: &TrackListModel, row: i32) -> i64;

        #[qinvokable]
        #[cxx_name = "analyzeSelected"]
        fn analyze_selected(self: &TrackListModel, force: bool);

        #[qinvokable]
        #[cxx_name = "importSelected"]
        fn import_selected(self: &TrackListModel, analyze: bool);

        /// 0xRRGGBB, or −1 to clear.
        #[qinvokable]
        #[cxx_name = "setColorSelected"]
        fn set_color_selected(self: &TrackListModel, color: i32);

        #[qinvokable]
        #[cxx_name = "removeSelected"]
        fn remove_selected(self: &TrackListModel);

        #[qinvokable]
        #[cxx_name = "addSelectedToPlaylist"]
        fn add_selected_to_playlist(self: &TrackListModel, playlist: i64);

        #[qinvokable]
        #[cxx_name = "resetGridSelected"]
        fn reset_grid_selected(self: &TrackListModel);

        /// 0..5 stars for every selected track.
        #[qinvokable]
        #[cxx_name = "setRatingSelected"]
        fn set_rating_selected(self: &TrackListModel, stars: i32);

        /// Path token of the folder holding the file of `row`.
        #[qinvokable]
        #[cxx_name = "folderToken"]
        fn folder_token(self: &TrackListModel, row: i32) -> i64;

        /// "Artist – Title" of `row`, or "N tracks" when several are selected.
        #[qinvokable]
        #[cxx_name = "selectionTitle"]
        fn selection_title(self: &TrackListModel, row: i32) -> QString;

        /// Fetches the shown Beatport list again.
        #[qinvokable]
        #[cxx_name = "reloadBeatport"]
        fn reload_beatport(self: &TrackListModel);

        /// The Beatport page of `row`'s track, or empty.
        #[qinvokable]
        #[cxx_name = "beatportUrl"]
        fn beatport_url(self: &TrackListModel, row: i32) -> QString;

        /// Downloads the selected Beatport tracks (all of the list when
        /// none is selected) and keeps them offline.
        #[qinvokable]
        #[cxx_name = "downloadBeatport"]
        fn download_beatport(self: &TrackListModel);

        /// The selected streamed tracks are no longer kept offline; their
        /// files are deleted.
        #[qinvokable]
        #[cxx_name = "removeDownloadsSelected"]
        fn remove_downloads_selected(self: &TrackListModel);

        /// Beatport tracks of the list not downloaded yet (all of them, or
        /// the selected ones).
        #[qinvokable]
        #[cxx_name = "notDownloadedCount"]
        fn not_downloaded_count(self: &TrackListModel, selected_only: bool) -> i32;

        /// Repaints the rows' download indicators (no reset, keeps the
        /// scroll position).
        #[qinvokable]
        #[cxx_name = "updateDownloads"]
        fn update_downloads(self: Pin<&mut TrackListModel>);
    }

    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut TrackListModel>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut TrackListModel>);
        #[inherit]
        fn index(self: &TrackListModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut TrackListModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

        #[cxx_override]
        fn data(self: &TrackListModel, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &TrackListModel) -> QHash_i32_QByteArray;
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &TrackListModel, parent: &QModelIndex) -> i32;
    }
}

use core::pin::Pin;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use rille_app::{AnalysisState, App, BeatportList, DownloadState, SortKey, Source};
use rille_core::GridFlags;
use rille_library::{CoverSize, TrackRow};

use crate::deck_controller::key_color;
use crate::global::{app, changed_since, changes_seq, path_token, token_path};
use crate::tree_model::{
    BP_OFFLINE, BP_PURCHASES, BP_TOP100, KIND_BEATPORT_LIST, KIND_BEATPORT_PLAYLIST, KIND_BEATPORT_RECENT,
    KIND_BEATPORT_SEARCH,
};

const TRACK_ROLES: &[&str] = &[
    "trackId",
    "title",
    "artist",
    "album",
    "label",
    "genre",
    "bpm",
    "keyText",
    "keyColor",
    "rating",
    "duration",
    "fileName",
    "cover",
    "analyzed",
    "gridNote",
    "playCount",
    "deckMark",
    "remixer",
    "comment",
    "missing",
    "rowNumber",
    "tagColor",
    "analysisState",
    "gridAttention",
    "inCollection",
    "locked",
    "selected",
    "year",
    "bitrate",
    "sampleRate",
    "fileSize",
    "dateAdded",
    "lastPlayed",
    "filePath",
    "match",
    "coverLarge",
    "beatportId",
    "streamed",
    "offlineState",
    "downloadProgress",
];

#[derive(Default)]
pub struct TrackListRust {
    search: QString,
    source_kind: i32,
    source_id: i64,
    sort_key: QString,
    descending: bool,
    count: i32,
    selected_count: i32,
    new_count: i32,
    summary: QString,
    rows: Vec<TrackRow>,
    /// Selected rows by file path (stable across refreshes and re-sorting).
    selected: HashSet<PathBuf>,
    /// Last clicked row, the start of a Shift range.
    anchor: Option<usize>,
    /// Last change sequence number seen (see `global::push_changed`).
    seen: u64,
    /// Suggestion scores (0..=1) by track id, in the suggestions view.
    scores: HashMap<i64, f32>,
}

fn roles(names: &[&str]) -> QHash<QHashPair_i32_QByteArray> {
    let mut h = QHash::<QHashPair_i32_QByteArray>::default();
    for (i, n) in names.iter().enumerate() {
        h.insert(256 + i as i32, QByteArray::from(*n));
    }
    h
}

fn sort_key(s: &str) -> SortKey {
    match s {
        "title" => SortKey::Title,
        "album" => SortKey::Album,
        "label" => SortKey::Label,
        "genre" => SortKey::Genre,
        "bpm" => SortKey::Bpm,
        "key" => SortKey::Key,
        "rating" => SortKey::Rating,
        "duration" => SortKey::Duration,
        "added" => SortKey::Added,
        "plays" => SortKey::PlayCount,
        "file" => SortKey::FileName,
        "remixer" => SortKey::Remixer,
        "comment" => SortKey::Comment,
        "year" => SortKey::Year,
        "bitrate" => SortKey::Bitrate,
        "samplerate" => SortKey::SampleRate,
        "size" => SortKey::FileSize,
        "played" => SortKey::LastPlayed,
        "path" => SortKey::Path,
        _ => SortKey::Artist,
    }
}

fn fmt_time(secs: f64) -> String {
    if secs <= 0.0 {
        return String::new();
    }
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn state_text(s: AnalysisState) -> &'static str {
    match s {
        AnalysisState::NotInCollection => "new",
        AnalysisState::NotAnalyzed => "none",
        AnalysisState::Stale => "stale",
        AnalysisState::Queued => "queued",
        AnalysisState::Running => "running",
        AnalysisState::Done => "done",
        AnalysisState::Failed => "failed",
    }
}

/// A streamed track kept offline with its file here.
fn is_offline(r: &TrackRow) -> bool {
    r.id >= 0 && r.beatport_offline && !r.missing
}

/// A Beatport track's availability for the row's indicator: "" (not from
/// Beatport), "stream" (stream only), "cached" (file here, may make room),
/// "offline" (kept offline), "queued" or "downloading".
fn offline_state(app: &App, r: &TrackRow) -> &'static str {
    let Some(bp) = r.beatport_id else { return "" };
    match app.beatport_download_state(bp) {
        DownloadState::Downloading(_) => "downloading",
        DownloadState::Queued => "queued",
        DownloadState::Idle if is_offline(r) => "offline",
        DownloadState::Idle if r.id >= 0 && !r.missing => "cached",
        DownloadState::Idle => "stream",
    }
}

/// Whether the grid should be checked by the DJ: tempo or bar start flagged.
fn grid_attention(r: &TrackRow) -> &'static str {
    let f = r.grid_flags();
    if f.contains(GridFlags::OCTAVE_AMBIGUOUS) {
        "check tempo"
    } else if f.contains(GridFlags::DOWNBEAT_UNCERTAIN) {
        "check bar start"
    } else if f.contains(GridFlags::LOW_CONFIDENCE) {
        "check grid"
    } else {
        ""
    }
}

impl TrackListRust {
    fn selected_rows(&self) -> Vec<&TrackRow> {
        self.rows.iter().filter(|r| self.selected.contains(&r.path)).collect()
    }

    fn selected_ids(&self) -> Vec<i64> {
        self.selected_rows().iter().filter(|r| r.id >= 0).map(|r| r.id).collect()
    }

    /// The Beatport list shown: the search box searches Beatport; playlists
    /// and charts are filtered locally.
    fn beatport_list(&self) -> Option<BeatportList> {
        match (self.source_kind, self.source_id) {
            (KIND_BEATPORT_SEARCH, _) => Some(BeatportList::Search(self.search.to_string().trim().to_owned())),
            (KIND_BEATPORT_PLAYLIST, id) => Some(BeatportList::Playlist(id)),
            (KIND_BEATPORT_LIST, BP_TOP100) => Some(BeatportList::Top100),
            (KIND_BEATPORT_LIST, BP_PURCHASES) => Some(BeatportList::Purchases),
            (KIND_BEATPORT_LIST, genre) => Some(BeatportList::Genre(genre)),
            _ => None,
        }
    }
}

impl qobject::TrackListModel {
    fn refresh(mut self: Pin<&mut Self>) {
        let Some(app) = app() else { return };
        let search = self.search().to_string();
        let beatport = self.rust().beatport_list();
        if let Some(list) = &beatport {
            // Fetched in the background unless already shown.
            app.beatport_open(list.clone(), false);
        }
        let mut rows = match *self.source_kind() {
            KIND_BEATPORT_SEARCH | KIND_BEATPORT_PLAYLIST | KIND_BEATPORT_LIST => {
                app.beatport_rows(beatport.as_ref().expect("beatport list"))
            }
            KIND_BEATPORT_RECENT => app.tracks(
                if *self.source_id() == BP_OFFLINE { Source::BeatportOffline } else { Source::BeatportRecent },
                &search,
                sort_key(&self.sort_key().to_string()),
                *self.descending(),
            ),
            1 => app.tracks(
                Source::Playlist(*self.source_id()),
                &search,
                sort_key(&self.sort_key().to_string()),
                *self.descending(),
            ),
            2 => app.tracks(
                Source::History(*self.source_id()),
                &search,
                sort_key(&self.sort_key().to_string()),
                *self.descending(),
            ),
            3 => token_path(*self.source_id()).map(|dir| app.folder_rows(&dir)).unwrap_or_default(),
            6 => app.tracks(Source::Suggestions, &search, sort_key(&self.sort_key().to_string()), *self.descending()),
            _ => app.tracks(Source::Collection, &search, sort_key(&self.sort_key().to_string()), *self.descending()),
        };
        if *self.source_kind() == 3 {
            // Folders: filter by the search text here, sort by the column.
            let needle = search.to_lowercase();
            if !needle.is_empty() {
                rows.retain(|r| {
                    [&r.title, &r.artist, &r.label, &r.genre].iter().any(|f| f.to_lowercase().contains(&needle))
                        || r.path.to_string_lossy().to_lowercase().contains(&needle)
                });
            }
            if self.sort_key().to_string() != "file" || *self.descending() {
                rille_app::sort_rows(&mut rows, sort_key(&self.sort_key().to_string()), *self.descending());
            }
        }
        if matches!(*self.source_kind(), KIND_BEATPORT_PLAYLIST | KIND_BEATPORT_LIST) {
            let needle = search.to_lowercase();
            if !needle.is_empty() {
                rows.retain(|r| {
                    [&r.title, &r.artist, &r.label, &r.genre].iter().any(|f| f.to_lowercase().contains(&needle))
                });
            }
        }
        if beatport.is_some() && (self.sort_key().to_string() != "artist" || *self.descending()) {
            // Beatport's order (relevance, playlist order) unless sorted.
            rille_app::sort_rows(&mut rows, sort_key(&self.sort_key().to_string()), *self.descending());
        }
        let total_secs: f64 = rows.iter().map(|r| r.duration_secs).sum();
        let new = rows.iter().filter(|r| r.id < 0).count();
        let scores: HashMap<i64, f32> = if *self.source_kind() == 6 {
            app.suggestions().into_iter().map(|s| (s.id, s.score)).collect()
        } else {
            HashMap::new()
        };
        let tracks = format!("{} tracks, {:.1} hours", rows.len(), total_secs / 3600.0 + 0.0);
        let summary = match (*self.source_kind(), &beatport) {
            (3, _) => format!("{} files, {} not in the collection", rows.len(), new),
            (6, _) => match app.suggestion_reference() {
                Some((deck, r)) => format!(
                    "{} tracks fitting deck {}: {}",
                    rows.len(),
                    char::from(b'A' + deck),
                    if r.artist.is_empty() { r.title } else { format!("{} – {}", r.artist, r.title) }
                ),
                None => "Load or play a track to get suggestions".into(),
            },
            (_, Some(list)) => {
                let status = app.beatport_status(list);
                match (status.loading, status.error, list) {
                    (true, _, _) => "Loading from Beatport…".to_string(),
                    (_, Some(e), _) => e,
                    (_, None, BeatportList::Search(q)) if q.is_empty() => {
                        "Search Beatport, or paste a beatport.com link".to_string()
                    }
                    _ => tracks,
                }
            }
            _ => tracks,
        };
        let seen = changes_seq();
        // The same tracks in the same order (e.g. a track was loaded or
        // imported): update the rows in place, so the view keeps its scroll
        // position and current row. A Beatport track keeps its place when it
        // turns from a catalog row into a collection row (another path).
        let same = {
            let old = &self.rust().rows;
            let same_track = |a: &TrackRow, b: &TrackRow| match (a.beatport_id, b.beatport_id) {
                (Some(x), Some(y)) => x == y,
                _ => a.path == b.path,
            };
            old.len() == rows.len() && old.iter().zip(&rows).all(|(a, b)| same_track(a, b))
        };
        if !same {
            self.as_mut().begin_reset_model();
        }
        {
            let mut r = self.as_mut().rust_mut();
            let paths: HashSet<PathBuf> = rows.iter().map(|x| x.path.clone()).collect();
            r.selected.retain(|p| paths.contains(p));
            r.rows = rows;
            if !same {
                r.anchor = None;
            }
            r.seen = seen;
            r.scores = scores;
        }
        if same {
            self.as_mut().emit_all_changed();
        } else {
            self.as_mut().end_reset_model();
        }
        let n = self.rust().rows.len() as i32;
        self.as_mut().set_count(n);
        self.as_mut().set_new_count(new as i32);
        let sel = self.rust().selected.len() as i32;
        self.as_mut().set_selected_count(sel);
        self.as_mut().set_summary(QString::from(summary));
    }

    fn update_changed(mut self: Pin<&mut Self>) {
        let Some(app) = app() else { return };
        let (ids, seq, lost) = changed_since(self.rust().seen);
        if lost {
            self.refresh();
            return;
        }
        self.as_mut().rust_mut().seen = seq;
        let ids: HashSet<i64> = ids.into_iter().collect();
        if ids.is_empty() {
            return;
        }
        let changed: Vec<usize> =
            self.rust().rows.iter().enumerate().filter(|(_, r)| ids.contains(&r.id)).map(|(i, _)| i).collect();
        for i in changed {
            let id = self.rust().rows[i].id;
            if let Some(row) = app.track_row(id) {
                self.as_mut().rust_mut().rows[i] = row;
            }
            self.as_mut().emit_row_changed(i);
        }
    }

    fn emit_row_changed(mut self: Pin<&mut Self>, row: usize) {
        let idx = self.index(row as i32, 0, &QModelIndex::default());
        self.as_mut().data_changed(&idx, &idx, &QList::<i32>::default());
    }

    fn emit_all_changed(mut self: Pin<&mut Self>) {
        let n = self.rust().rows.len() as i32;
        if n == 0 {
            return;
        }
        let (a, b) = (self.index(0, 0, &QModelIndex::default()), self.index(n - 1, 0, &QModelIndex::default()));
        self.as_mut().data_changed(&a, &b, &QList::<i32>::default());
    }

    fn track_id(&self, row: i32) -> i64 {
        self.rust().rows.get(row.max(0) as usize).map_or(-1, |r| r.id)
    }

    fn path_token(&self, row: i32) -> i64 {
        self.rust().rows.get(row.max(0) as usize).map_or(-1, |r| path_token(&r.path))
    }

    fn sort_by(mut self: Pin<&mut Self>, key: &QString) {
        if *self.sort_key() == *key {
            let d = !*self.descending();
            self.as_mut().set_descending(d);
        } else {
            self.as_mut().set_sort_key(key.clone());
            self.as_mut().set_descending(false);
        }
        self.refresh();
    }

    fn select(mut self: Pin<&mut Self>, row: i32, mode: i32) {
        let Ok(row) = usize::try_from(row) else { return };
        if row >= self.rust().rows.len() {
            return;
        }
        {
            let mut r = self.as_mut().rust_mut();
            let path = r.rows[row].path.clone();
            match mode {
                1 => {
                    if !r.selected.remove(&path) {
                        r.selected.insert(path);
                    }
                    r.anchor = Some(row);
                }
                2 => {
                    let a = r.anchor.unwrap_or(row);
                    let (lo, hi) = (a.min(row), a.max(row));
                    let paths: Vec<PathBuf> = r.rows[lo..=hi].iter().map(|x| x.path.clone()).collect();
                    r.selected.clear();
                    r.selected.extend(paths);
                }
                _ => {
                    r.selected.clear();
                    r.selected.insert(path);
                    r.anchor = Some(row);
                }
            }
        }
        let n = self.rust().selected.len() as i32;
        self.as_mut().set_selected_count(n);
        self.emit_all_changed();
    }

    fn select_all(mut self: Pin<&mut Self>) {
        {
            let mut r = self.as_mut().rust_mut();
            let paths: Vec<PathBuf> = r.rows.iter().map(|x| x.path.clone()).collect();
            r.selected.extend(paths);
        }
        let n = self.rust().selected.len() as i32;
        self.as_mut().set_selected_count(n);
        self.emit_all_changed();
    }

    fn is_selected(&self, row: i32) -> bool {
        self.rust().rows.get(row.max(0) as usize).is_some_and(|r| self.rust().selected.contains(&r.path))
    }

    fn load_row_to_cell(&self, row: i32, deck: i32, cell: i32) {
        let (Some(app), Some(r)) = (app(), self.rust().rows.get(row.max(0) as usize)) else { return };
        let (deck, cell) = (deck.clamp(0, 3) as u8, usize::try_from(cell).ok());
        match (r.id, r.beatport_id) {
            (id, _) if id >= 0 => app.load_remix_cell(deck, cell, id),
            // A catalog track joins the collection first, then fills a free cell.
            (_, Some(bp)) => app.load_beatport(deck, bp),
            _ => app.load_remix_file(deck, cell, &r.path),
        }
    }

    fn load_row(&self, row: i32, deck: i32) {
        let (Some(app), Some(r)) = (app(), self.rust().rows.get(row.max(0) as usize)) else { return };
        let deck = deck.clamp(0, 3) as u8;
        match (r.id, r.beatport_id) {
            (id, _) if id >= 0 => app.load_track(deck, id),
            (_, Some(bp)) => app.load_beatport(deck, bp),
            _ => app.load_file(deck, &r.path),
        }
    }

    fn analyze_selected(&self, force: bool) {
        let Some(app) = app() else { return };
        // Catalog tracks are analyzed when they are loaded.
        let paths: Vec<PathBuf> = self
            .rust()
            .selected_rows()
            .iter()
            .filter(|r| r.id >= 0 || r.beatport_id.is_none())
            .map(|r| r.path.clone())
            .collect();
        app.analyze_paths(paths, force);
    }

    fn import_selected(&self, analyze: bool) {
        let Some(app) = app() else { return };
        let rows = self.rust().selected_rows();
        let new_file = |r: &TrackRow| r.id < 0 && r.beatport_id.is_none();
        let paths: Vec<PathBuf> = if rows.is_empty() {
            // Nothing selected: the whole folder.
            self.rust().rows.iter().filter(|r| new_file(r)).map(|r| r.path.clone()).collect()
        } else {
            rows.iter().filter(|r| new_file(r)).map(|r| r.path.clone()).collect()
        };
        if !paths.is_empty() {
            app.import_paths(paths, analyze);
        }
    }

    fn set_color_selected(&self, color: i32) {
        let Some(app) = app() else { return };
        let c = u32::try_from(color).ok().filter(|c| *c <= 0xFF_FFFF);
        app.set_track_color(&self.rust().selected_ids(), c);
    }

    /// Streamed tracks also leave the cache.
    fn remove_selected(&self) {
        let Some(app) = app() else { return };
        let rows = self.rust().selected_rows();
        let (streamed, local): (Vec<&TrackRow>, Vec<&TrackRow>) =
            rows.into_iter().filter(|r| r.id >= 0).partition(|r| r.beatport_id.is_some());
        if !streamed.is_empty() {
            app.remove_streamed(&streamed.iter().map(|r| r.id).collect::<Vec<_>>());
        }
        if !local.is_empty() {
            app.remove_tracks(&local.iter().map(|r| r.id).collect::<Vec<_>>());
        }
    }

    fn reload_beatport(&self) {
        let (Some(app), Some(list)) = (app(), self.rust().beatport_list()) else { return };
        app.beatport_open(list, true);
    }

    fn beatport_url(&self, row: i32) -> QString {
        let bp = self.rust().rows.get(row.max(0) as usize).and_then(|r| r.beatport_id);
        QString::from(bp.map_or_else(String::new, |id| format!("https://www.beatport.com/track/-/{id}")))
    }

    fn download_beatport(&self) {
        let Some(app) = app() else { return };
        let r = self.rust();
        let rows = if r.selected.is_empty() { r.rows.iter().collect() } else { r.selected_rows() };
        let ids: Vec<i64> = rows.iter().filter(|r| !is_offline(r)).filter_map(|r| r.beatport_id).collect();
        if !ids.is_empty() {
            app.beatport_download(ids);
        }
    }

    fn remove_downloads_selected(&self) {
        let Some(app) = app() else { return };
        let ids: Vec<i64> =
            self.rust().selected_rows().iter().filter(|r| r.id >= 0 && r.beatport_id.is_some()).map(|r| r.id).collect();
        app.beatport_remove_downloads(&ids);
    }

    fn not_downloaded_count(&self, selected_only: bool) -> i32 {
        let r = self.rust();
        let rows: Vec<&TrackRow> = if selected_only { r.selected_rows() } else { r.rows.iter().collect() };
        rows.iter().filter(|r| r.beatport_id.is_some() && !is_offline(r)).count() as i32
    }

    fn update_downloads(mut self: Pin<&mut Self>) {
        // Rows whose download finished are collection rows now.
        if let Some(app) = app() {
            for i in 0..self.rust().rows.len() {
                let id = self.rust().rows[i].id;
                if id >= 0
                    && let Some(row) = app.track_row(id)
                {
                    self.as_mut().rust_mut().rows[i] = row;
                }
            }
        }
        self.emit_all_changed();
    }

    fn add_selected_to_playlist(&self, playlist: i64) {
        let Some(app) = app() else { return };
        app.add_to_playlist(playlist, &self.rust().selected_ids());
    }

    fn set_rating_selected(&self, stars: i32) {
        let Some(app) = app() else { return };
        for id in self.rust().selected_ids() {
            app.set_rating(id, stars.clamp(0, 5) as u8);
        }
    }

    fn folder_token(&self, row: i32) -> i64 {
        self.rust().rows.get(row.max(0) as usize).and_then(|r| r.path.parent()).map_or(-1, path_token)
    }

    fn selection_title(&self, row: i32) -> QString {
        let n = self.rust().selected.len();
        if n > 1 {
            return QString::from(format!("{n} tracks"));
        }
        let t = self.rust().rows.get(row.max(0) as usize).map_or(String::new(), |r| {
            if r.artist.is_empty() { r.title.clone() } else { format!("{} – {}", r.artist, r.title) }
        });
        QString::from(t)
    }

    fn reset_grid_selected(&self) {
        let Some(app) = app() else { return };
        app.reset_grids(&self.rust().selected_ids());
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let i = index.row().max(0) as usize;
        let Some(r) = self.rust().rows.get(i) else { return QVariant::default() };
        let Some(app) = app() else { return QVariant::default() };
        let name = TRACK_ROLES.get((role - 256).max(0) as usize).copied().unwrap_or("");
        let s = |t: &str| QVariant::from(&QString::from(t));
        match name {
            "trackId" => QVariant::from(&r.id),
            "title" => s(&r.title),
            "artist" => s(&r.artist),
            "album" => s(&r.album),
            "label" => s(&r.label),
            "genre" => s(&r.genre),
            "remixer" => s(&r.remixer),
            "comment" => s(&r.comment),
            "bpm" => s(&r.bpm.map_or(String::new(), |b| format!("{b:.2}"))),
            "keyText" => s(&r.key.map_or(String::new(), |k| app.settings().key_text(k))),
            "keyColor" => s(&r.key.map_or("#8a9099".into(), key_color)),
            "rating" => QVariant::from(&i32::from(r.rating)),
            "duration" => s(&fmt_time(r.duration_secs)),
            "fileName" => s(&r.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
            "cover" | "coverLarge" => {
                let (size, px) = if name == "cover" { (CoverSize::Small, 100) } else { (CoverSize::Large, 500) };
                s(&r.cover
                    .as_deref()
                    .and_then(|c| app.cover_file(c, size))
                    .map(|p| format!("file://{}", p.display()))
                    // Catalog tracks: Beatport's image.
                    .or_else(|| r.beatport_id.filter(|_| r.id < 0).and_then(|bp| app.beatport_cover_url(bp, px)))
                    .unwrap_or_default())
            }
            "analyzed" => QVariant::from(&r.analyzed),
            "gridNote" | "gridAttention" => s(if r.analyzed { grid_attention(r) } else { "" }),
            "playCount" => QVariant::from(&(r.play_count as i32)),
            // A streamed track out of the cache is downloaded again, not lost.
            "missing" => QVariant::from(&(r.missing && r.beatport_id.is_none())),
            "deckMark" => {
                let marks: String = (0..4u8)
                    .filter(|d| r.id >= 0 && app.deck(*d).track_id == Some(r.id))
                    .map(|d| char::from(b'A' + d))
                    .collect();
                s(&marks)
            }
            "rowNumber" => QVariant::from(&(i as i32 + 1)),
            "tagColor" => s(&r.color.map_or(String::new(), |c| format!("#{c:06x}"))),
            "analysisState" => s(state_text(app.analysis_state(r))),
            "inCollection" => QVariant::from(&(r.id >= 0)),
            "locked" => QVariant::from(&r.grid_locked),
            "selected" => QVariant::from(&self.rust().selected.contains(&r.path)),
            "year" => s(&r.year.map_or(String::new(), |y| y.to_string())),
            "bitrate" => s(&r.bitrate.map_or(String::new(), |b| format!("{b} kbps"))),
            "sampleRate" => s(&r.sample_rate.map_or(String::new(), |h| format!("{:.1} kHz", f64::from(h) / 1000.0))),
            "fileSize" => {
                s(&if r.file_size > 0 { format!("{:.1} MB", r.file_size as f64 / 1_048_576.0) } else { String::new() })
            }
            "dateAdded" => s(&if r.date_added > 0 {
                crate::tree_model::format_date(r.date_added)[..10].to_string()
            } else {
                String::new()
            }),
            "lastPlayed" => s(&r.last_played.map_or(String::new(), crate::tree_model::format_date)),
            "filePath" => s(&r.path.display().to_string()),
            "match" => s(&self.rust().scores.get(&r.id).map_or(String::new(), |m| format!("{:.0} %", m * 100.0))),
            "beatportId" => QVariant::from(&r.beatport_id.unwrap_or(-1)),
            "streamed" => QVariant::from(&r.beatport_id.is_some()),
            "offlineState" => s(offline_state(app, r)),
            "downloadProgress" => QVariant::from(&match r.beatport_id.map(|b| app.beatport_download_state(b)) {
                Some(DownloadState::Downloading(f)) => f64::from(f),
                _ => -1.0,
            }),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        roles(TRACK_ROLES)
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rust().rows.len() as i32
    }
}
