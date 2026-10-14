//! Beatport streaming: browsing the catalog (search, links, the user's
//! playlists) and loading streamed tracks. A loaded track becomes a
//! collection row marked with its Beatport id, its file kept in the cache,
//! so cues, grids and analysis persist like for any other track.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use rille_beatport::{Arrived, Client, Named, Playlist, Quality, Track};
use rille_library::{Tags, TrackId, TrackRow};

use crate::{App, MAX_DECKS, UiEvent};

/// What the Beatport part of the browser lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeatportList {
    /// Search text, or a pasted beatport.com link.
    Search(String),
    /// One of the user's Beatport playlists.
    Playlist(i64),
    /// Beatport's Top 100 overall.
    Top100,
    /// The Top 100 of a genre.
    Genre(i64),
    /// The user's purchases that can be streamed.
    Purchases,
    /// An artist's tracks, newest first.
    Artist(i64),
    /// A label's tracks, newest first.
    Label(i64),
}

#[derive(Default)]
struct View {
    list: Option<BeatportList>,
    tracks: Vec<Track>,
    loading: bool,
    error: Option<String>,
    /// Long lists come in pages: the next one to fetch, if any.
    next_page: Option<u32>,
    /// All tracks of the list, when Beatport says.
    total: Option<u64>,
}

pub(crate) struct BeatportState {
    pub(crate) client: Client,
    view: RwLock<View>,
    /// Bumped per request; answers to older ones are dropped.
    seq: AtomicU64,
    /// `None` until fetched.
    playlists: RwLock<Option<Vec<Playlist>>>,
    playlists_loading: AtomicBool,
    /// `None` until fetched.
    genres: RwLock<Option<Vec<Named>>>,
    genres_loading: AtomicBool,
    /// Catalog data of every listed track, for loading it.
    known: RwLock<HashMap<i64, Track>>,
    queue: Queue,
    /// One lock per track being downloaded, so it downloads once.
    inflight: Mutex<HashMap<i64, Arc<Mutex<()>>>>,
}

impl BeatportState {
    pub(crate) fn new(token_path: PathBuf) -> Self {
        Self {
            client: Client::new(token_path),
            view: RwLock::default(),
            seq: AtomicU64::new(0),
            playlists: RwLock::new(None),
            playlists_loading: AtomicBool::new(false),
            genres: RwLock::new(None),
            genres_loading: AtomicBool::new(false),
            known: RwLock::default(),
            queue: Queue::default(),
            inflight: Mutex::default(),
        }
    }
}

/// A streamed track's download, as a deck shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Download {
    /// 0..1 (0 while the size is unknown).
    pub fraction: f32,
    pub total_bytes: u64,
    /// Average since the start.
    pub bytes_per_sec: f64,
    pub remaining_secs: Option<f64>,
}

impl Download {
    /// "3.4 MB/s · 9 s left".
    pub fn describe(&self) -> String {
        let speed = format!("{:.1} MB/s", self.bytes_per_sec / 1_048_576.0);
        match self.remaining_secs {
            Some(s) if self.bytes_per_sec > 0.0 => format!("{speed} · {} s left", s.ceil() as u64),
            _ => speed,
        }
    }
}

/// The browser's state of a Beatport list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeatportStatus {
    pub loading: bool,
    pub error: Option<String>,
    /// More pages can be fetched ([`App::beatport_load_more`]).
    pub more: bool,
    /// All tracks of the list, when Beatport says (search: all matches).
    pub total: Option<u64>,
}

impl App {
    /// The signed-in Beatport user, `None` when signed out.
    pub fn beatport_account(&self) -> Option<String> {
        self.beatport.client.account()
    }

    /// Signs in to Beatport in the background.
    pub fn beatport_login(self: &Arc<Self>, username: String, password: String) {
        let app = self.clone();
        std::thread::Builder::new()
            .name("beatport-login".into())
            .spawn(move || {
                app.notify(UiEvent::Status("Signing in to Beatport…".into()));
                match app.beatport.client.login(&username, &password) {
                    Ok(()) => {
                        app.beatport_reset();
                        app.notify(UiEvent::Status(format!("Signed in to Beatport as {}", username.trim())));
                    }
                    Err(e) => app.notify(UiEvent::Status(e.to_string())),
                }
                app.notify(UiEvent::BeatportChanged);
            })
            .expect("spawn beatport login");
    }

    pub fn beatport_logout(&self) {
        self.beatport.client.logout();
        self.beatport_reset();
        self.notify(UiEvent::Status("Signed out of Beatport".into()));
        self.notify(UiEvent::BeatportChanged);
    }

    /// Forgets listed results (signed in or out).
    fn beatport_reset(&self) {
        self.beatport.seq.fetch_add(1, Ordering::Relaxed);
        *self.beatport.view.write().expect("beatport lock") = View::default();
        *self.beatport.playlists.write().expect("beatport lock") = None;
        *self.beatport.genres.write().expect("beatport lock") = None;
    }

    /// Shows `list`: fetches it in the background unless it is already
    /// shown (or being fetched). `reload` fetches it again anyway.
    pub fn beatport_open(self: &Arc<Self>, list: BeatportList, reload: bool) {
        {
            let view = self.beatport.view.read().expect("beatport lock");
            if !reload && view.list.as_ref() == Some(&list) {
                return;
            }
        }
        let seq = self.beatport.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let empty = matches!(&list, BeatportList::Search(q) if q.trim().is_empty());
        *self.beatport.view.write().expect("beatport lock") =
            View { list: Some(list.clone()), loading: !empty, ..View::default() };
        self.notify(UiEvent::BeatportChanged);
        if !empty {
            self.fetch_page(list, 1, seq);
        }
    }

    /// The next page of the shown list, if it has one and none is coming.
    pub fn beatport_load_more(self: &Arc<Self>) {
        let (list, page) = {
            let mut view = self.beatport.view.write().expect("beatport lock");
            let (Some(list), Some(page)) = (view.list.clone(), view.next_page) else { return };
            if view.loading {
                return;
            }
            view.loading = true;
            (list, page)
        };
        self.notify(UiEvent::BeatportChanged);
        self.fetch_page(list, page, self.beatport.seq.load(Ordering::Relaxed));
    }

    /// Fetches page `page` of `list` in the background and adds it to the
    /// view (unless another list was opened meanwhile).
    fn fetch_page(self: &Arc<Self>, list: BeatportList, page: u32, seq: u64) {
        let app = self.clone();
        std::thread::Builder::new()
            .name("beatport-list".into())
            .spawn(move || {
                let result = app.list_page(&list, page);
                if app.beatport.seq.load(Ordering::Relaxed) != seq {
                    return;
                }
                let mut view = app.beatport.view.write().expect("beatport lock");
                view.loading = false;
                match result {
                    Ok(p) => {
                        let mut known = app.beatport.known.write().expect("beatport lock");
                        for t in &p.tracks {
                            known.insert(t.id, t.clone());
                        }
                        drop(known);
                        // Pages can overlap when the catalog changes meanwhile.
                        let have: HashSet<i64> = view.tracks.iter().map(|t| t.id).collect();
                        view.tracks.extend(p.tracks.into_iter().filter(|t| !have.contains(&t.id)));
                        view.next_page = p.more.then_some(page + 1);
                        view.total = p.total;
                    }
                    Err(e) => view.error = Some(e),
                }
                drop(view);
                app.notify(UiEvent::BeatportChanged);
            })
            .expect("spawn beatport list");
    }

    /// One page of a list; lists that are not paged come whole as page 1.
    fn list_page(&self, list: &BeatportList, page: u32) -> Result<rille_beatport::Page, String> {
        let client = &self.beatport.client;
        let whole = |r: rille_beatport::Result<Vec<Track>>| {
            r.map(|tracks| rille_beatport::Page { tracks, total: None, more: false }).map_err(|e| e.to_string())
        };
        match list {
            BeatportList::Search(q) if rille_beatport::looks_like_link(q) => {
                match rille_beatport::parse_link(q)? {
                    // An artist or label link: all their tracks, page by page.
                    rille_beatport::Link::Artist(id) => self.list_page(&BeatportList::Artist(id), page),
                    rille_beatport::Link::Label(id) => self.list_page(&BeatportList::Label(id), page),
                    link => whole(client.link_tracks(link)),
                }
            }
            BeatportList::Search(q) => client.search_page(q, page).map_err(|e| e.to_string()),
            BeatportList::Artist(id) => client.artist_tracks(*id, page).map_err(|e| e.to_string()),
            BeatportList::Label(id) => client.label_tracks(*id, page).map_err(|e| e.to_string()),
            BeatportList::Playlist(id) => whole(client.playlist_tracks(*id)),
            BeatportList::Top100 => whole(client.top100(None)),
            BeatportList::Genre(id) => whole(client.top100(Some(*id))),
            BeatportList::Purchases => whole(client.purchases()),
        }
    }

    /// The artists and the label of a Beatport track, for links to their
    /// tracks. A track not listed in this session is looked up in the
    /// background ([`UiEvent::BeatportChanged`] when known).
    pub fn beatport_links(self: &Arc<Self>, beatport_id: i64) -> Option<BeatportLinks> {
        let known = self.beatport.known.read().expect("beatport lock").get(&beatport_id).cloned();
        let Some(t) = known else {
            if self.beatport_account().is_some() {
                let app = self.clone();
                std::thread::Builder::new()
                    .name("beatport-track".into())
                    .spawn(move || {
                        if let Ok(t) = app.beatport.client.track(beatport_id) {
                            app.beatport.known.write().expect("beatport lock").insert(beatport_id, t);
                            app.notify(UiEvent::BeatportChanged);
                        }
                    })
                    .expect("spawn beatport track");
            }
            return None;
        };
        let artists = t.artists.iter().chain(&t.remixers).map(|a| (a.id, a.name.clone())).collect::<Vec<_>>();
        let mut seen = HashSet::new();
        let artists = artists.into_iter().filter(|(id, _)| seen.insert(*id)).collect();
        let label = t.release.as_ref().and_then(|r| r.label.as_ref()).map(|l| (l.id, l.name.clone()));
        Some(BeatportLinks { artists, label })
    }

    /// Rows of `list` for the browser (empty until it is fetched; see
    /// [`Self::beatport_open`]). Tracks streamed before are their collection
    /// rows; the others have id −1.
    pub fn beatport_rows(&self, list: &BeatportList) -> Vec<TrackRow> {
        let view = self.beatport.view.read().expect("beatport lock");
        if view.list.as_ref() != Some(list) {
            return Vec::new();
        }
        let t = self.tracks.read().expect("tracks lock");
        let streamed: HashMap<i64, usize> =
            t.rows.iter().enumerate().filter_map(|(i, r)| r.beatport_id.map(|b| (b, i))).collect();
        view.tracks
            .iter()
            .map(|track| match streamed.get(&track.id) {
                Some(&i) => t.rows[i].clone(),
                None => self.catalog_row(track),
            })
            .collect()
    }

    pub fn beatport_status(&self, list: &BeatportList) -> BeatportStatus {
        let view = self.beatport.view.read().expect("beatport lock");
        if view.list.as_ref() != Some(list) {
            return BeatportStatus::default();
        }
        BeatportStatus {
            loading: view.loading,
            error: view.error.clone(),
            more: view.next_page.is_some(),
            total: view.total,
        }
    }

    /// The user's Beatport playlists; the first call (signed in) fetches
    /// them in the background.
    pub fn beatport_playlists(self: &Arc<Self>) -> Vec<Playlist> {
        if let Some(p) = self.beatport.playlists.read().expect("beatport lock").as_ref() {
            return p.clone();
        }
        if self.beatport_account().is_some() {
            self.beatport_refresh_playlists();
        }
        Vec::new()
    }

    pub fn beatport_refresh_playlists(self: &Arc<Self>) {
        if self.beatport.playlists_loading.swap(true, Ordering::Relaxed) {
            return;
        }
        let app = self.clone();
        std::thread::Builder::new()
            .name("beatport-playlists".into())
            .spawn(move || {
                let result = app.beatport.client.my_playlists();
                app.beatport.playlists_loading.store(false, Ordering::Relaxed);
                match result {
                    Ok(p) => *app.beatport.playlists.write().expect("beatport lock") = Some(p),
                    Err(e) => {
                        // Not retried until asked (the tree would ask every refresh).
                        *app.beatport.playlists.write().expect("beatport lock") = Some(Vec::new());
                        app.notify(UiEvent::Status(format!("Beatport playlists: {e}")));
                    }
                }
                app.notify(UiEvent::BeatportChanged);
            })
            .expect("spawn beatport playlists");
    }

    /// Beatport's genres (for their Top 100s); the first call (signed in)
    /// fetches them in the background.
    pub fn beatport_genres(self: &Arc<Self>) -> Vec<Named> {
        if let Some(g) = self.beatport.genres.read().expect("beatport lock").as_ref() {
            return g.clone();
        }
        if self.beatport_account().is_some() && !self.beatport.genres_loading.swap(true, Ordering::Relaxed) {
            let app = self.clone();
            std::thread::Builder::new()
                .name("beatport-genres".into())
                .spawn(move || {
                    let result = app.beatport.client.genres();
                    app.beatport.genres_loading.store(false, Ordering::Relaxed);
                    if let Err(e) = &result {
                        app.notify(UiEvent::Status(format!("Beatport genres: {e}")));
                    }
                    // A failure is not retried until signing in again.
                    *app.beatport.genres.write().expect("beatport lock") = Some(result.unwrap_or_default());
                    app.notify(UiEvent::BeatportChanged);
                })
                .expect("spawn beatport genres");
        }
        Vec::new()
    }

    /// Loads a Beatport track onto a deck: it joins the collection as a
    /// streamed track, then loads like any other (downloading its file).
    pub fn load_beatport(self: &Arc<Self>, deck: u8, beatport_id: i64) {
        if self.refuse_load(deck) {
            return;
        }
        let app = self.clone();
        std::thread::Builder::new()
            .name(format!("beatport-load-{deck}"))
            .spawn(move || match app.add_streamed(beatport_id) {
                Ok(id) => app.load_track(deck, id),
                Err(e) => app.notify(UiEvent::Status(format!("Beatport: {e}"))),
            })
            .expect("spawn beatport load");
    }

    /// The collection row of a Beatport track, created (with its cover) from
    /// the catalog if needed.
    fn add_streamed(&self, beatport_id: i64) -> Result<TrackId, String> {
        let id = self.add_streamed_with(beatport_id, true)?;
        self.refresh_tracks();
        Ok(id)
    }

    /// [`Self::add_streamed`]; `with_cover` false leaves fetching the cover
    /// for later (many tracks at once), and the caller refreshes the rows.
    fn add_streamed_with(&self, beatport_id: i64, with_cover: bool) -> Result<TrackId, String> {
        let known = self.beatport.known.read().expect("beatport lock").get(&beatport_id).cloned();
        let existing = self.library.lock().expect("library lock").streamed_track(beatport_id).ok().flatten();
        let track = match (known, existing) {
            (Some(t), _) => t,
            (None, Some(id)) => return Ok(id),
            (None, None) => self.beatport.client.track(beatport_id).map_err(|e| e.to_string())?,
        };
        // The cover once (again if an earlier fetch failed).
        let has_cover = existing.and_then(|id| self.track_row(id)).is_some_and(|r| r.has_cover);
        let cover = match has_cover || !with_cover {
            true => None,
            false => track.cover_url(500).and_then(|u| self.beatport.client.fetch_image(&u, 8 << 20).ok()),
        };
        self.library
            .lock()
            .expect("library lock")
            .upsert_streamed(beatport_id, &self.planned_file(beatport_id), &catalog_tags(&track), cover.as_deref())
            .map_err(|e| e.to_string())
    }

    /// Where a track's file goes in the cache, by the quality setting (the
    /// download decides in the end).
    fn planned_file(&self, beatport_id: i64) -> PathBuf {
        let ext = match Quality::from_name(&self.settings().beatport_quality) {
            Quality::Lossless => "flac",
            _ => "m4a",
        };
        self.paths.beatport_cache().join(format!("{beatport_id}.{ext}"))
    }

    /// A browser row for a catalog track not streamed yet.
    fn catalog_row(&self, t: &Track) -> TrackRow {
        let tags = catalog_tags(t);
        TrackRow {
            id: -1,
            path: self.planned_file(t.id),
            title: tags.title,
            artist: tags.artist,
            album: tags.album,
            remixer: tags.remixer,
            label: tags.label,
            genre: tags.genre,
            comment: String::new(),
            year: tags.year,
            duration_secs: tags.duration_secs,
            bpm: tags.bpm,
            key: tags.key,
            rating: 0,
            color: None,
            play_count: 0,
            last_played: None,
            date_added: 0,
            file_size: 0,
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
            beatport_id: Some(t.id),
            beatport_offline: false,
            guest: false,
        }
    }

    /// Cover image URL of a listed catalog track (rows not streamed yet
    /// have no cached thumbnail).
    pub fn beatport_cover_url(&self, beatport_id: i64, px: u32) -> Option<String> {
        self.beatport.known.read().expect("beatport lock").get(&beatport_id)?.cover_url(px)
    }

    /// A streamed track's file, downloaded first if it is not in the cache.
    /// `deck` = `(deck, load sequence)` shows the progress there and stops
    /// when the deck loads something else. `arrived` follows the download,
    /// to read the file while it comes (not when another download of the
    /// track is already running: this one then waits for it).
    pub(crate) fn streamed_file(
        &self,
        row: &TrackRow,
        deck: Option<(u8, u64)>,
        arrived: Option<&Arrived>,
    ) -> Result<PathBuf, String> {
        self.streamed_file_to(row, deck.map_or(Sink::None, |(d, seq)| Sink::Deck(d, seq)), arrived)
    }

    /// Only one download per track runs at a time: a second caller waits for
    /// it (a deck loading a track the queue is fetching shows that progress).
    fn streamed_file_to(&self, row: &TrackRow, sink: Sink, arrived: Option<&Arrived>) -> Result<PathBuf, String> {
        let Some(beatport_id) = row.beatport_id else { return Ok(row.path.clone()) };
        if row.path.exists() {
            // Recently used: last to leave the cache.
            let _ =
                std::fs::File::options().append(true).open(&row.path).and_then(|f| f.set_modified(SystemTime::now()));
            return Ok(row.path.clone());
        }
        let gate = self.beatport.inflight.lock().expect("inflight lock").entry(beatport_id).or_default().clone();
        let _turn = loop {
            match gate.try_lock() {
                Ok(turn) => break turn,
                Err(std::sync::TryLockError::Poisoned(p)) => break p.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    let other = self.beatport.queue.active.lock().expect("queue lock").get(&beatport_id).copied();
                    if !sink.show(self, Some(other.unwrap_or_default())) {
                        return Err("cancelled".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        };
        // The other download may have brought the file.
        let now = self.library.lock().expect("library lock").track(row.id).ok().flatten();
        if let Some(r) = now.as_ref().filter(|r| r.path.exists()) {
            sink.show(self, None);
            return Ok(r.path.clone());
        }
        let now = now.unwrap_or_else(|| row.clone());
        // Shown as "connecting" until the first bytes arrive.
        sink.show(self, Some(Download::default()));
        let quality = Quality::from_name(&self.settings().beatport_quality);
        let d = match self.beatport.client.download_location(beatport_id, quality) {
            Ok(d) => d,
            Err(e) => {
                sink.show(self, None);
                return Err(e.to_string());
            }
        };
        // Kept offline: a readable file in the downloads folder; else the cache.
        let dest = if now.beatport_offline {
            self.download_path(&now, d.extension())
        } else {
            self.paths.beatport_cache().join(format!("{beatport_id}.{}", d.extension()))
        };
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let mut progress = |p: rille_beatport::Progress| {
            let secs = started.elapsed().as_secs_f64().max(0.001);
            let bytes_per_sec = p.done as f64 / secs;
            let download = Download {
                fraction: p.fraction().unwrap_or(0.0),
                total_bytes: p.total,
                bytes_per_sec,
                // Only once the speed means something.
                remaining_secs: (p.total > 0 && secs > 1.0 && bytes_per_sec > 0.0)
                    .then(|| p.total.saturating_sub(p.done) as f64 / bytes_per_sec),
            };
            if !sink.show(self, Some(download)) {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let result = self.beatport.client.fetch_file(&d.location, &dest, &cancel, &mut progress, arrived);
        sink.show(self, None);
        result.map_err(|e| e.to_string())?;
        let _ = self.library.lock().expect("library lock").set_streamed_file(row.id, &dest);
        self.refresh_track(row.id);
        self.prune_beatport_cache(&dest);
        Ok(dest)
    }

    /// Cached files, oldest use first: `(last use, size, path)`.
    fn cache_files(&self) -> Vec<(SystemTime, u64, PathBuf)> {
        let Ok(dir) = std::fs::read_dir(self.paths.beatport_cache()) else { return Vec::new() };
        let mut files: Vec<_> = dir
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x != "part"))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                m.is_file().then(|| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), e.path()))
            })
            .collect();
        files.sort();
        files
    }

    /// A free file name for a downloaded track: "Artist - Title (Mix).flac"
    /// in the downloads folder.
    fn download_path(&self, row: &TrackRow, ext: &str) -> PathBuf {
        let dir = self.paths.beatport_downloads();
        let name = if row.artist.is_empty() { row.title.clone() } else { format!("{} - {}", row.artist, row.title) };
        // No path separators or characters other systems refuse; not too long.
        let mut name: String = name
            .chars()
            .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c })
            .collect::<String>()
            .trim()
            .trim_start_matches('.')
            .chars()
            .take(180)
            .collect();
        if name.is_empty() {
            name = format!("Beatport {}", row.beatport_id.unwrap_or(row.id));
        }
        let mut path = dir.join(format!("{name}.{ext}"));
        // Two tracks with the same name: the second gets its Beatport id.
        if path.exists() && path != row.path {
            path = dir.join(format!("{name} [{}].{ext}", row.beatport_id.unwrap_or(row.id)));
        }
        path
    }

    /// Keeps streamed tracks offline: never removed to make room, their
    /// files moved from the cache to the downloads folder (readable names).
    fn keep_offline(&self, ids: &[TrackId]) {
        let _ = self.library.lock().expect("library lock").set_streamed_offline(ids, true);
        let cache = self.paths.beatport_cache();
        for &id in ids {
            let Some(row) = self.library.lock().expect("library lock").track(id).ok().flatten() else { continue };
            if !row.path.exists() || !row.path.starts_with(&cache) {
                continue;
            }
            let ext = row.path.extension().and_then(|e| e.to_str()).unwrap_or("flac").to_owned();
            let dest = self.download_path(&row, &ext);
            if move_file(&row.path, &dest).is_ok() {
                let _ = self.library.lock().expect("library lock").set_streamed_file(id, &dest);
                // A deck playing it reads the file again only when loaded again.
                for d in 0..MAX_DECKS as u8 {
                    if self.deck(d).path.as_deref() == Some(row.path.as_path()) {
                        let seq = self.deck(d).engine_id;
                        self.update_deck(d, seq, |i| i.path = Some(dest.clone()));
                    }
                }
            }
        }
        self.refresh_tracks();
    }

    /// Moves downloads still in the cache (from before the downloads folder)
    /// to the downloads folder; at start, in the background.
    pub(crate) fn relocate_downloads(&self) {
        let cache = self.paths.beatport_cache();
        let ids: Vec<TrackId> = {
            let t = self.tracks.read().expect("tracks lock");
            t.rows.iter().filter(|r| r.beatport_offline && r.path.starts_with(&cache)).map(|r| r.id).collect()
        };
        if !ids.is_empty() {
            self.keep_offline(&ids);
            self.notify(UiEvent::Status(format!(
                "Beatport: moved {} downloads to {}",
                ids.len(),
                self.paths.beatport_downloads().display()
            )));
        }
    }

    /// Files of the streamed tracks kept offline.
    fn offline_files(&self) -> HashSet<PathBuf> {
        let t = self.tracks.read().expect("tracks lock");
        t.rows.iter().filter(|r| r.beatport_offline && r.beatport_id.is_some()).map(|r| r.path.clone()).collect()
    }

    /// Keeps the cache within its size: the least recently used files go,
    /// never one kept offline, loaded on a deck, or `keep`. Downloads left
    /// unfinished (rille quit meanwhile) go after an hour.
    fn prune_beatport_cache(&self, keep: &Path) {
        if let Ok(dir) = std::fs::read_dir(self.paths.beatport_cache()) {
            let stale = |e: &std::fs::DirEntry| {
                e.path().extension().is_some_and(|x| x == "part")
                    && e.metadata()
                        .and_then(|m| m.modified())
                        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > std::time::Duration::from_secs(3600)))
            };
            for e in dir.flatten().filter(stale) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let limit = u64::from(self.settings().beatport_cache_mb) << 20;
        let files = self.cache_files();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= limit {
            return;
        }
        let offline = self.offline_files();
        let loaded: Vec<PathBuf> = (0..MAX_DECKS as u8).filter_map(|d| self.deck(d).path).collect();
        let mut evicted = Vec::new();
        for (_, size, path) in files {
            if total <= limit {
                break;
            }
            if path == keep || offline.contains(&path) || loaded.contains(&path) || std::fs::remove_file(&path).is_err()
            {
                continue;
            }
            total -= size;
            evicted.push(path);
        }
        self.forget_cached(&evicted);
    }

    /// Deletes the cached files of streamed tracks (`with_offline`: also
    /// the downloads kept offline), except those loaded on a deck.
    pub fn clear_beatport_cache(&self, with_offline: bool) {
        let loaded: Vec<PathBuf> = (0..MAX_DECKS as u8).filter_map(|d| self.deck(d).path).collect();
        let offline = self.offline_files();
        let mut evicted = Vec::new();
        for (_, _, path) in self.cache_files() {
            let keep = loaded.contains(&path) || (!with_offline && offline.contains(&path));
            if !keep && std::fs::remove_file(&path).is_ok() {
                evicted.push(path);
            }
        }
        if with_offline {
            let rows: Vec<TrackRow> = {
                let t = self.tracks.read().expect("tracks lock");
                t.rows.iter().filter(|r| r.beatport_offline && !loaded.contains(&r.path)).cloned().collect()
            };
            for r in &rows {
                if std::fs::remove_file(&r.path).is_ok() {
                    evicted.push(r.path.clone());
                }
            }
            let ids: Vec<TrackId> = rows.iter().map(|r| r.id).collect();
            let _ = self.library.lock().expect("library lock").set_streamed_offline(&ids, false);
        }
        self.forget_cached(&evicted);
        self.notify(UiEvent::Status(format!("Beatport: removed {} downloaded files", evicted.len())));
        self.notify(UiEvent::BeatportChanged);
    }

    /// Downloads Beatport tracks in the background and keeps them offline
    /// (tracks already in the cache are just kept). Two at a time, each in
    /// parallel parts.
    pub fn beatport_download(self: &Arc<Self>, beatport_ids: Vec<i64>) {
        if self.beatport_account().is_none() {
            self.notify(UiEvent::Status("Sign in to Beatport to download tracks".into()));
            return;
        }
        let q = &self.beatport.queue;
        // Tracks with their file here only need to be kept.
        let (present, wanted): (Vec<TrackRow>, Vec<i64>) = {
            let t = self.tracks.read().expect("tracks lock");
            let by_bp: HashMap<i64, &TrackRow> = t.rows.iter().filter_map(|r| r.beatport_id.map(|b| (b, r))).collect();
            let mut present = Vec::new();
            let mut wanted = Vec::new();
            for bp in beatport_ids {
                match by_bp.get(&bp) {
                    Some(r) if !r.missing && r.path.exists() => present.push((*r).clone()),
                    _ => wanted.push(bp),
                }
            }
            (present, wanted)
        };
        if !present.is_empty() {
            // Into the downloads folder (cached files move there).
            let ids: Vec<TrackId> = present.iter().filter(|r| !r.beatport_offline).map(|r| r.id).collect();
            self.keep_offline(&ids);
        }
        let added = {
            let mut jobs = q.jobs.lock().expect("queue lock");
            let active = q.active.lock().expect("queue lock");
            if q.workers.load(Ordering::Relaxed) == 0 && jobs.is_empty() {
                // A new batch.
                q.total.store(0, Ordering::Relaxed);
                q.done.store(0, Ordering::Relaxed);
                q.failed.store(0, Ordering::Relaxed);
                q.cancel.store(false, Ordering::Relaxed);
            }
            let mut added = 0;
            for bp in wanted {
                if !jobs.contains(&bp) && !active.contains_key(&bp) {
                    jobs.push_back(bp);
                    added += 1;
                }
            }
            q.total.fetch_add(added, Ordering::Relaxed);
            added
        };
        let kept = present.len();
        self.notify(UiEvent::Status(match (added, kept) {
            (0, 0) => "Beatport: these tracks are already downloading".into(),
            (0, k) => format!("Beatport: {k} tracks were downloaded already and stay offline"),
            (n, _) => format!("Beatport: downloading {n} tracks for offline use"),
        }));
        self.notify(UiEvent::BeatportDownloads);
        // Workers start and stop under the jobs lock, so none is missing
        // while jobs wait.
        let start = {
            let jobs = q.jobs.lock().expect("queue lock");
            let running = q.workers.load(Ordering::Relaxed);
            let start = DOWNLOAD_WORKERS.saturating_sub(running).min(jobs.len());
            q.workers.fetch_add(start, Ordering::Relaxed);
            start
        };
        for _ in 0..start {
            let app = self.clone();
            std::thread::Builder::new()
                .name("beatport-download".into())
                .spawn(move || app.download_worker())
                .expect("spawn beatport download");
        }
    }

    fn download_worker(self: Arc<Self>) {
        let q = &self.beatport.queue;
        let last = loop {
            let job = {
                let mut jobs = q.jobs.lock().expect("queue lock");
                if q.cancel.load(Ordering::Relaxed) || self.shutdown.load(Ordering::Relaxed) {
                    jobs.clear();
                }
                match jobs.pop_front() {
                    Some(bp) => {
                        q.active.lock().expect("queue lock").insert(bp, Download::default());
                        Ok(bp)
                    }
                    None => Err(q.workers.fetch_sub(1, Ordering::Relaxed) == 1),
                }
            };
            let bp = match job {
                Ok(bp) => bp,
                Err(last) => break last,
            };
            let result = self.download_one(bp);
            q.active.lock().expect("queue lock").remove(&bp);
            match result {
                Ok(()) => q.done.fetch_add(1, Ordering::Relaxed),
                Err(e) => {
                    if !q.cancel.load(Ordering::Relaxed) {
                        self.notify(UiEvent::Status(format!("Beatport download: {e}")));
                    }
                    q.failed.fetch_add(1, Ordering::Relaxed)
                }
            };
            self.notify(UiEvent::BeatportDownloads);
        };
        // The last worker reports the batch.
        if last {
            let (done, failed) = (q.done.load(Ordering::Relaxed), q.failed.load(Ordering::Relaxed));
            let text = if q.cancel.load(Ordering::Relaxed) {
                format!("Beatport: downloads stopped after {done} tracks")
            } else if failed > 0 {
                format!("Beatport: downloaded {done} tracks for offline use, {failed} failed")
            } else {
                format!("Beatport: downloaded {done} tracks for offline use")
            };
            self.notify(UiEvent::Status(text));
            self.notify(UiEvent::BeatportDownloads);
        }
    }

    /// One queued track: into the collection, kept offline, downloaded.
    fn download_one(&self, beatport_id: i64) -> Result<(), String> {
        let id = self.add_streamed(beatport_id)?;
        // Kept offline first, so the file goes to the downloads folder.
        self.keep_offline(&[id]);
        let row = self.library.lock().expect("library lock").track(id).ok().flatten().ok_or("track gone")?;
        self.streamed_file_to(&row, Sink::Queue(beatport_id), None).map(|_| ())
    }

    /// Downloads a whole list (a playlist from the tree) for offline use. A
    /// playlist also becomes a local playlist under "Beatport" (updated when
    /// downloaded again), so it is there without Beatport.
    pub fn beatport_download_list(self: &Arc<Self>, list: BeatportList) {
        let app = self.clone();
        std::thread::Builder::new()
            .name("beatport-download-list".into())
            .spawn(move || {
                // The first page of a paged list (its tracks as listed).
                let tracks = match app.list_page(&list, 1) {
                    Ok(p) => p.tracks,
                    Err(e) => return app.notify(UiEvent::Status(format!("Beatport: {e}"))),
                };
                let mut known = app.beatport.known.write().expect("beatport lock");
                for t in &tracks {
                    known.insert(t.id, t.clone());
                }
                drop(known);
                if let BeatportList::Playlist(bp) = list
                    && let Err(e) = app.copy_playlist(bp, &tracks)
                {
                    app.notify(UiEvent::Status(format!("Beatport playlist: {e}")));
                }
                app.beatport_download(tracks.iter().map(|t| t.id).collect());
            })
            .expect("spawn beatport download list");
    }

    /// The local copy of Beatport playlist `bp`: a playlist of the same name
    /// in the "Beatport" folder, holding its tracks in Beatport's order.
    fn copy_playlist(&self, bp: i64, tracks: &[Track]) -> Result<(), String> {
        let name = self
            .beatport
            .playlists
            .read()
            .expect("beatport lock")
            .as_ref()
            .and_then(|p| p.iter().find(|p| p.id == bp).map(|p| p.name.clone()))
            .unwrap_or_else(|| format!("Beatport playlist {bp}"));
        // Rows for every track now (covers come with the downloads).
        let ids = tracks.iter().map(|t| self.add_streamed_with(t.id, false)).collect::<Result<Vec<_>, _>>()?;
        let mut lib = self.library.lock().expect("library lock");
        let list = match lib.beatport_playlist(bp).map_err(|e| e.to_string())? {
            Some(list) => list,
            None => {
                let folder = lib.playlist_named(None, "Beatport", true).map_err(|e| e.to_string())?;
                lib.playlist_named(Some(folder), &name, false).map_err(|e| e.to_string())?
            }
        };
        lib.set_playlist_tracks(list, &ids).map_err(|e| e.to_string())?;
        lib.set_beatport_playlist(bp, list).map_err(|e| e.to_string())?;
        drop(lib);
        self.refresh_tracks();
        Ok(())
    }

    /// A downloaded Beatport playlist: `(tracks downloaded, tracks)` of its
    /// local copy, `None` when it was not downloaded.
    pub fn beatport_playlist_offline(&self, bp: i64) -> Option<(usize, usize)> {
        let ids = {
            let lib = self.library.lock().expect("library lock");
            let list = lib.beatport_playlist(bp).ok().flatten()?;
            lib.playlist_tracks(list).ok()?
        };
        let t = self.tracks.read().expect("tracks lock");
        let offline = ids
            .iter()
            .filter(|id| t.by_id.get(id).is_some_and(|&i| t.rows[i].beatport_offline && !t.rows[i].missing))
            .count();
        Some((offline, ids.len()))
    }

    /// Lets a downloaded Beatport playlist go: its tracks no longer kept
    /// offline (files deleted unless another downloaded playlist has them)
    /// and its local copy deleted.
    pub fn beatport_remove_playlist_download(&self, bp: i64) {
        let (list, ids, others) = {
            let lib = self.library.lock().expect("library lock");
            let Some(list) = lib.beatport_playlist(bp).ok().flatten() else { return };
            let ids = lib.playlist_tracks(list).unwrap_or_default();
            let others: HashSet<TrackId> = lib
                .beatport_playlists()
                .unwrap_or_default()
                .into_iter()
                .filter(|(other, _)| *other != bp)
                .flat_map(|(_, l)| lib.playlist_tracks(l).unwrap_or_default())
                .collect();
            (list, ids, others)
        };
        let only_here: Vec<TrackId> = ids.into_iter().filter(|id| !others.contains(id)).collect();
        self.beatport_remove_downloads(&only_here);
        let _ = self.library.lock().expect("library lock").delete_playlist(list);
        self.refresh_tracks();
        self.notify(UiEvent::BeatportChanged);
    }

    /// The folder with the downloaded files.
    pub fn beatport_downloads_folder(&self) -> PathBuf {
        self.paths.beatport_downloads()
    }

    pub fn beatport_cancel_downloads(&self) {
        self.beatport.queue.cancel.store(true, Ordering::Relaxed);
        self.beatport.queue.jobs.lock().expect("queue lock").clear();
        self.notify(UiEvent::BeatportDownloads);
    }

    /// The download queue, for the status bar.
    pub fn beatport_downloads(&self) -> DownloadsStatus {
        let q = &self.beatport.queue;
        let active = q.active.lock().expect("queue lock");
        DownloadsStatus {
            total: q.total.load(Ordering::Relaxed),
            done: q.done.load(Ordering::Relaxed),
            failed: q.failed.load(Ordering::Relaxed),
            running: q.workers.load(Ordering::Relaxed) > 0,
            bytes_per_sec: active.values().map(|d| d.bytes_per_sec).sum(),
            current: active.values().map(|d| d.fraction).fold(0.0, f32::max),
        }
    }

    /// Where a Beatport track stands in the download queue.
    pub fn beatport_download_state(&self, beatport_id: i64) -> DownloadState {
        let q = &self.beatport.queue;
        if let Some(d) = q.active.lock().expect("queue lock").get(&beatport_id) {
            return DownloadState::Downloading(d.fraction);
        }
        if q.jobs.lock().expect("queue lock").contains(&beatport_id) {
            return DownloadState::Queued;
        }
        DownloadState::Idle
    }

    /// Lets streamed tracks go: no longer kept offline, files deleted
    /// (except on a deck). They stay in the collection with their cues.
    pub fn beatport_remove_downloads(&self, ids: &[TrackId]) {
        let loaded: Vec<PathBuf> = (0..MAX_DECKS as u8).filter_map(|d| self.deck(d).path).collect();
        let rows: Vec<TrackRow> =
            ids.iter().filter_map(|&id| self.track_row(id)).filter(|r| r.beatport_id.is_some()).collect();
        let _ = self
            .library
            .lock()
            .expect("library lock")
            .set_streamed_offline(&rows.iter().map(|r| r.id).collect::<Vec<_>>(), false);
        let evicted: Vec<PathBuf> = rows
            .iter()
            .filter(|r| !loaded.contains(&r.path) && std::fs::remove_file(&r.path).is_ok())
            .map(|r| r.path.clone())
            .collect();
        self.forget_cached(&evicted);
        self.refresh_tracks();
        self.notify(UiEvent::BeatportChanged);
    }

    /// Number of streamed tracks kept offline with their file here.
    pub fn offline_count(&self) -> usize {
        let t = self.tracks.read().expect("tracks lock");
        t.rows.iter().filter(|r| r.beatport_offline && r.beatport_id.is_some() && !r.missing).count()
    }

    /// Marks the rows of deleted cache files missing.
    fn forget_cached(&self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        {
            let mut lib = self.library.lock().expect("library lock");
            for p in paths {
                if let Ok(Some(id)) = lib.track_by_path(p) {
                    let _ = lib.set_streamed_file(id, p);
                }
            }
        }
        self.refresh_tracks();
    }

    /// Removes streamed tracks from the collection and deletes their files.
    pub fn remove_streamed(&self, ids: &[TrackId]) {
        let rows: Vec<TrackRow> = ids.iter().filter_map(|&id| self.track_row(id)).collect();
        let streamed: Vec<TrackId> = rows.iter().filter(|r| r.beatport_id.is_some()).map(|r| r.id).collect();
        for r in rows.iter().filter(|r| r.beatport_id.is_some()) {
            let _ = std::fs::remove_file(&r.path);
        }
        self.remove_tracks(&streamed);
    }

    /// What the cache (streamed tracks) and the downloads folder hold.
    pub fn beatport_cache_usage(&self) -> CacheUsage {
        let offline = self.offline_files();
        let mut u = CacheUsage::default();
        for (_, size, path) in self.cache_files() {
            if !offline.contains(&path) {
                u.files += 1;
                u.bytes += size;
            }
        }
        for path in &offline {
            if let Ok(m) = std::fs::metadata(path) {
                u.offline_files += 1;
                u.offline_bytes += m.len();
            }
        }
        u
    }
}

/// A track's artists (and remixers) and label as `(Beatport id, name)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BeatportLinks {
    pub artists: Vec<(i64, String)>,
    pub label: Option<(i64, String)>,
}

/// Where a download shows its progress.
#[derive(Clone, Copy)]
enum Sink {
    None,
    /// The deck and load sequence: the download stops when it loads
    /// something else.
    Deck(u8, u64),
    /// The download queue, for this Beatport id.
    Queue(i64),
}

impl Sink {
    /// Shows `d` (`None` when done); false when the download should stop.
    fn show(self, app: &App, d: Option<Download>) -> bool {
        match self {
            Self::None => true,
            Self::Deck(deck, seq) => app.update_deck(deck, seq, |i| i.download = d),
            Self::Queue(bp) => {
                let q = &app.beatport.queue;
                if let Some(d) = d {
                    q.active.lock().expect("queue lock").insert(bp, d);
                }
                // The browser's indicators, a few times a second.
                let mut last = q.last_notify.lock().expect("queue lock");
                if last.is_none_or(|t| t.elapsed().as_millis() >= 300) {
                    *last = Some(std::time::Instant::now());
                    app.notify(UiEvent::BeatportDownloads);
                }
                !q.cancel.load(Ordering::Relaxed)
            }
        }
    }
}

/// Parallel downloads of the queue (each in parallel parts itself).
const DOWNLOAD_WORKERS: usize = 2;

#[derive(Default)]
struct Queue {
    jobs: Mutex<VecDeque<i64>>,
    /// Downloads running now, by Beatport id.
    active: Mutex<HashMap<i64, Download>>,
    total: AtomicUsize,
    done: AtomicUsize,
    failed: AtomicUsize,
    workers: AtomicUsize,
    cancel: AtomicBool,
    last_notify: Mutex<Option<std::time::Instant>>,
}

/// The download queue, for the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DownloadsStatus {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub running: bool,
    pub bytes_per_sec: f64,
    /// The furthest running download, 0..1.
    pub current: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DownloadState {
    Idle,
    Queued,
    /// 0..1.
    Downloading(f32),
}

/// What the Beatport cache and downloads folder hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheUsage {
    /// Streamed tracks in the cache (within the storage limit).
    pub files: usize,
    pub bytes: u64,
    /// Downloads kept offline.
    pub offline_files: usize,
    pub offline_bytes: u64,
}

/// Renames `from` to `to`, copying across file systems.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

/// Collection tags from the catalog.
fn catalog_tags(t: &Track) -> Tags {
    Tags {
        title: t.display_title(),
        artist: t.artist_names(),
        album: t.release.as_ref().map_or_else(String::new, |r| r.name.clone()),
        remixer: t.remixer_names(),
        label: t.label_name(),
        genre: t.genre_name(),
        comment: String::new(),
        year: t.year(),
        bpm: t.bpm.filter(|b| *b > 0.0),
        key: t.key.as_ref().and_then(|k| k.camelot()).and_then(|c| rille_library::parse_key(&c)),
        duration_secs: t.duration_secs(),
        bitrate: None,
        sample_rate: None,
    }
}
