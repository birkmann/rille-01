//! Application core without any UI: owns the engine, the library, track
//! loading and analysis, MIDI and settings. The Qt UI (and tests) drive it
//! through [`App`].

pub mod analysis;
mod audio;
pub mod beatport;
pub mod drums;
mod engine_slot;
pub mod explorer;
mod folder_meta;
pub mod record;
pub mod remix;
pub mod settings;
pub mod stems;
pub mod suggest;
pub mod timing;
mod values;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use rille_analysis::AnalysisConfig;
use rille_core::track::{ANALYZER_VERSION, HOTCUE_COLORS};
use rille_core::{
    BeatClock, BeatGrid, Control, ControlEvent, ControlValue, CuePoint, Key, Scope, TrackCues, WaveformSummary,
};
use rille_engine::backend::AudioConfig;
use rille_engine::{Command, Event, HOTCUES, Hotcue, LoadedTrack, MAX_DECKS, Snapshot, TrackAudio};
use rille_library::{CoverSize, Library, PlaylistId, ScanProgress, SearchIndex, TrackId, TrackRow};
use rille_midi::{LearnSession, Mapping, MappingStore, MidiEvent, MidiManager, port_base_name};

pub use analysis::{AnalysisQueue, AnalysisState, Priority};
pub use audio::AudioStatus;
pub use beatport::{BeatportList, BeatportStatus, CacheUsage, DownloadState, DownloadsStatus};
use engine_slot::EngineSlot;
use explorer::FolderTree;
pub use remix::{RemixCell, RemixSet};
pub use rille_beatport::Quality as BeatportQuality;
pub use rille_library::{HistorySession, PlaylistNode};
pub use settings::{KeyNotation, MixingMode, Paths, Settings, WaveformStyle};
pub use suggest::Suggestion;
pub use timing::{Timing, format_duration};

/// Something the UI should react to.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    /// Title, grid, waveform… of a deck changed.
    DeckChanged(u8),
    LibraryChanged,
    /// Only these rows changed (analysis finished, rating…).
    TracksChanged(Vec<TrackId>),
    Status(String),
    AnalysisProgress(AnalysisProgress),
    /// Import or scan: files read so far (all zero when finished).
    ImportProgress {
        done: usize,
        total: usize,
        timing: Timing,
    },
    /// Browser controls and "load selected" from a controller.
    Browser(ControlEvent),
    MidiChanged,
    MidiLearned(String),
    AudioChanged,
    /// The track suggestions are for changed (see [`App::suggestion_reference`]).
    SuggestionsChanged,
    /// Beatport sign-in, a list or the playlists changed.
    BeatportChanged,
    /// The download queue moved on (rows' indicators, the status bar).
    BeatportDownloads,
    /// The stem model's download moved on, or finished.
    StemsChanged,
    /// The drum machine's kit or kit list changed.
    DrumsChanged,
    /// Tags of more files in this explorer folder were read.
    FolderMetaChanged(PathBuf),
}

/// The analysis queue's state for the status bar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnalysisProgress {
    pub done: usize,
    pub failed: usize,
    pub total: usize,
    pub paused: bool,
    /// "Artist – Title" of a track being analyzed.
    pub current: String,
    pub timing: Timing,
}

impl AnalysisProgress {
    pub fn idle(&self) -> bool {
        self.done + self.failed >= self.total && self.current.is_empty()
    }
}

/// What a deck holds, for display.
#[derive(Clone, Debug, Default)]
pub struct DeckInfo {
    pub track_id: Option<TrackId>,
    /// Identity of this load inside the engine.
    pub engine_id: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub remixer: String,
    pub label: String,
    pub key: Option<Key>,
    pub grid: Option<Arc<BeatGrid>>,
    pub waveform: Option<Arc<WaveformSummary>>,
    /// The track's file and decoded audio (for capturing loops from it).
    pub path: Option<PathBuf>,
    pub audio: Option<Arc<TrackAudio>>,
    /// Set for a remix deck: its cells. `title` is the set's name.
    pub remix: Option<Arc<RemixSet>>,
    pub cover: Option<PathBuf>,
    /// The track streams from Beatport.
    pub streamed: bool,
    pub loading: bool,
    /// A streamed track is being downloaded.
    pub download: Option<beatport::Download>,
    pub stems: stems::DeckStems,
    pub analyzing: bool,
    pub error: Option<String>,
    /// Bumped on every change.
    pub revision: u64,
    play_secs: f64,
    logged: bool,
    taps: Vec<Instant>,
}

/// Corrections to a deck's beatgrid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridEdit {
    /// Move all beats (milliseconds, positive = later).
    Move(f64),
    DoubleTempo,
    HalveTempo,
    /// The beat nearest to the play position moves onto it.
    BeatHere,
    /// The beat nearest to the play position becomes a downbeat.
    DownbeatHere,
    /// Beat 1 of a bar on the play position (snapped to the kick there):
    /// for grids whose first beat misses the track's first kick.
    BarStartHere,
    /// Tap the tempo; four or more taps set the BPM.
    Tap,
    Lock(bool),
    /// Back to the analyzer's grid.
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Collection,
    Playlist(i64),
    History(i64),
    /// Tracks that fit the playing one, best first (see [`suggest`]).
    Suggestions,
    /// Tracks streamed from Beatport, most recent first.
    BeatportRecent,
    /// Streamed tracks kept offline (downloaded), most recent first.
    BeatportOffline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Artist,
    Title,
    Album,
    Label,
    Genre,
    Bpm,
    Key,
    Rating,
    Duration,
    Added,
    PlayCount,
    FileName,
    Remixer,
    Comment,
    Year,
    Bitrate,
    SampleRate,
    FileSize,
    LastPlayed,
    Path,
}

/// The track the suggestions are for (deck, track) and the tracks on the
/// decks, which they leave out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SuggestFor {
    reference: Option<(u8, TrackId)>,
    loaded: [Option<TrackId>; MAX_DECKS],
}

struct Tracks {
    rows: Vec<TrackRow>,
    index: SearchIndex,
    /// Row position by track id.
    by_id: HashMap<TrackId, usize>,
}

impl Tracks {
    fn new(rows: Vec<TrackRow>) -> Self {
        let index = SearchIndex::new(&rows);
        let by_id = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        Self { rows, index, by_id }
    }
}

/// Seconds of a streamed track decoded before it starts to play.
const STREAM_START_SECS: usize = 5;
/// A deck this close (seconds) to the end of the streamed audio it has gets
/// what is decoded so far, if that is at least `STREAM_EARLY_SECS` more.
const STREAM_NEAR_END_SECS: f64 = 15.0;
const STREAM_EARLY_SECS: usize = 10;

/// A streamed track for a deck: the whole audio and the downloaded file.
enum Streamed {
    Decoded(rille_decode::DecodedAudio, PathBuf),
    Failed(String),
}

pub struct App {
    pub paths: Paths,
    settings: RwLock<Settings>,
    engine: Arc<EngineSlot>,
    audio_status: RwLock<AudioStatus>,
    audio_runner: Mutex<Option<audio::AudioRunner>>,
    library: Mutex<Library>,
    tracks: RwLock<Tracks>,
    decks: [RwLock<DeckInfo>; MAX_DECKS],
    load_seq: AtomicU64,
    ui_tx: Sender<UiEvent>,
    ui_rx: Receiver<UiEvent>,
    /// Wakes the background planner (new files, settings changed).
    analysis_kick: Sender<()>,
    queue: Arc<AnalysisQueue>,
    history_session: Mutex<Option<i64>>,
    /// Tracks logged to this session's history (see `log_history`).
    played: RwLock<HashSet<TrackId>>,
    /// What the suggestions were made for, while they are switched on.
    suggest_for: Mutex<SuggestFor>,
    midi: Mutex<Option<MidiManager>>,
    mappings: Mutex<MappingStore>,
    /// Where mappings are loaded from: the user's folder, then the bundled.
    mapping_dirs: Vec<PathBuf>,
    learn: Arc<Mutex<Option<LearnSession>>>,
    last_tick: Mutex<Instant>,
    /// Dropouts already logged, and when.
    xruns_logged: Mutex<(Instant, u64)>,
    shutdown: Arc<AtomicBool>,
    beatport: beatport::BeatportState,
    recording: Mutex<Option<record::Recording>>,
    /// A recording is running (for controller LEDs).
    recording_on: Arc<AtomicBool>,
    stems: stems::StemsState,
    drums: Mutex<drums::Drums>,
    /// The drum machine panel is shown (for controller LEDs).
    drums_visible: Arc<AtomicBool>,
    /// The drum kits and the one loaded (for controller pads and screens).
    drum_kit_list: Arc<RwLock<values::KitList>>,
    /// The mounted drives last seen and when, to notice a USB stick
    /// plugged in or pulled out.
    drives: Mutex<(Instant, Vec<PathBuf>)>,
    /// Reads tags of explorer folders in the background.
    folder_meta: folder_meta::FolderMetaState,
    /// This app, for work that must outlive a `&self` call.
    me: Weak<App>,
}

/// Options for [`App::start`].
#[derive(Clone, Debug)]
pub struct StartOptions {
    pub paths: Paths,
    /// Open the audio device (off for tests and headless runs).
    pub audio: bool,
    pub midi: bool,
    /// Folder with bundled controller mappings.
    pub bundled_mappings: Option<PathBuf>,
}

impl App {
    pub fn start(opts: StartOptions) -> Result<Arc<App>, String> {
        let paths = opts.paths;
        paths.create().map_err(|e| format!("cannot create app folders: {e}"))?;
        let settings = Settings::load(&paths.settings_file());
        let library =
            Library::open(&paths.library_db(), &paths.cache).map_err(|e| format!("cannot open the library: {e}"))?;
        let rows = library.tracks().map_err(|e| format!("cannot read the library: {e}"))?;
        let (ui_tx, ui_rx) = crossbeam_channel::unbounded();
        let (kick_tx, kick_rx) = crossbeam_channel::unbounded();
        let mut mapping_dirs = vec![paths.user_mappings()];
        mapping_dirs.extend(opts.bundled_mappings.clone());
        let mappings = MappingStore::load_with_channels(&mapping_dirs, &settings.midi_channels);
        let drums_visible = Arc::new(AtomicBool::new(settings.drums_visible));
        let app = Arc::new_cyclic(|me| App {
            drums: Mutex::new(drums::Drums::load(&paths.drums())),
            drums_visible,
            drum_kit_list: Arc::default(),
            me: me.clone(),
            settings: RwLock::new(settings),
            engine: Arc::new(EngineSlot::default()),
            audio_status: RwLock::new(AudioStatus::default()),
            audio_runner: Mutex::new(None),
            library: Mutex::new(library),
            tracks: RwLock::new(Tracks::new(rows)),
            decks: std::array::from_fn(|_| RwLock::new(DeckInfo::default())),
            load_seq: AtomicU64::new(1),
            ui_tx,
            ui_rx,
            analysis_kick: kick_tx,
            queue: Arc::new(AnalysisQueue::default()),
            history_session: Mutex::new(None),
            played: RwLock::default(),
            suggest_for: Mutex::default(),
            midi: Mutex::new(None),
            mappings: Mutex::new(mappings),
            mapping_dirs,
            learn: Arc::new(Mutex::new(None)),
            last_tick: Mutex::new(Instant::now()),
            xruns_logged: Mutex::new((Instant::now(), 0)),
            shutdown: Arc::new(AtomicBool::new(false)),
            beatport: beatport::BeatportState::new(paths.beatport_token()),
            recording: Mutex::new(None),
            recording_on: Arc::new(AtomicBool::new(false)),
            stems: stems::StemsState::default(),
            folder_meta: folder_meta::FolderMetaState::default(),
            drives: Mutex::new((Instant::now(), explorer::drives())),
            paths,
        });
        app.restart_audio(opts.audio);
        app.apply_deck_types();
        if opts.midi && app.settings().midi {
            app.start_midi();
        }
        app.queue.set_paused(app.settings().analysis_paused);
        app.clone().spawn_analysis_workers(kick_rx);
        if app.settings().background_analysis {
            let _ = app.analysis_kick.send(());
        }
        Ok(app)
    }

    /// Stops background work and audio.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.stop_folder_meta();
        let _ = self.analysis_kick.send(());
        self.queue.shutdown();
        *self.midi.lock().expect("midi lock") = None;
        self.stop_recording();
        self.capture_drums();
        *self.audio_runner.lock().expect("audio lock") = None;
    }

    /// Every two seconds: a drive plugged in or pulled out refreshes the
    /// explorer.
    fn watch_drives(&self, now: Instant) {
        let mut drives = self.drives.lock().expect("drives lock");
        if now.duration_since(drives.0) < Duration::from_secs(2) {
            return;
        }
        drives.0 = now;
        let current = explorer::drives();
        if current != drives.1 {
            drives.1 = current;
            drop(drives);
            self.notify(UiEvent::LibraryChanged);
        }
    }

    fn is_shut_down(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    // ---------------------------------------------------------------- state

    pub fn settings(&self) -> Settings {
        self.settings.read().expect("settings lock").clone()
    }

    /// Applies and saves settings; restarts audio if the device changed.
    pub fn set_settings(self: &Arc<Self>, s: Settings) {
        let old = self.settings();
        *self.settings.write().expect("settings lock") = s.clone();
        let _ = s.save(&self.paths.settings_file());
        self.drums_visible.store(s.drums_visible, Ordering::Relaxed);
        if old.audio_device != s.audio_device || old.buffer_frames != s.buffer_frames {
            self.restart_audio(true);
        } else {
            self.send_engine_settings();
        }
        if old.library_roots != s.library_roots {
            self.sync_roots();
        }
        if old.remix_decks != s.remix_decks {
            self.apply_deck_types();
        }
        if !old.background_analysis && s.background_analysis {
            let _ = self.analysis_kick.send(());
        }
        if old.suggestions != s.suggestions {
            // The browser shows or hides its Suggestions entry.
            *self.suggest_for.lock().expect("suggest lock") = SuggestFor::default();
            self.notify(UiEvent::LibraryChanged);
        }
    }

    pub fn audio_status(&self) -> AudioStatus {
        self.audio_status.read().expect("audio status lock").clone()
    }

    pub fn output_devices(&self) -> Vec<String> {
        rille_engine::backend::output_devices()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.engine.get().map(|e| e.snapshot()).unwrap_or_default()
    }

    /// Audio dropouts (buffer underruns) since the output started.
    pub fn xruns(&self) -> u64 {
        self.engine.get().map_or(0, |e| e.xruns())
    }

    pub fn deck(&self, deck: u8) -> DeckInfo {
        self.decks[usize::from(deck).min(MAX_DECKS - 1)].read().expect("deck lock").clone()
    }

    /// The track on a deck, without copying the rest of [`DeckInfo`].
    pub fn deck_track(&self, deck: u8) -> Option<TrackId> {
        self.decks[usize::from(deck).min(MAX_DECKS - 1)].read().expect("deck lock").track_id
    }

    /// Whether a track counts as played in this session (it is in the
    /// session's history).
    pub fn played_in_session(&self, id: TrackId) -> bool {
        self.played.read().expect("played lock").contains(&id)
    }

    /// Events for the UI since the last call.
    pub fn poll_ui_events(&self) -> Vec<UiEvent> {
        self.ui_rx.try_iter().collect()
    }

    fn notify(&self, e: UiEvent) {
        let _ = self.ui_tx.send(e);
    }

    /// Sends a raw engine command (clock tempo, master selection…).
    pub fn command(&self, cmd: Command) {
        self.send(cmd);
    }

    fn send(&self, cmd: Command) {
        if let Some(e) = self.engine.get()
            && e.send(cmd).is_err()
        {
            eprintln!("engine command queue full");
        }
    }

    // ---------------------------------------------------------------- audio

    fn restart_audio(self: &Arc<Self>, device: bool) {
        // The recording belongs to the old engine.
        if self.stop_recording().is_some() {
            self.notify(UiEvent::Status("Recording stopped: the audio output changed".into()));
        }
        let s = self.settings();
        // Stop the old stream first so the device is free.
        *self.audio_runner.lock().expect("audio lock") = None;
        let cfg = AudioConfig { device: s.audio_device.clone(), buffer_frames: s.buffer_frames };
        let app = Arc::downgrade(self);
        let (handle, status, runner) = audio::start(&cfg, device, move |status| {
            if let Some(app) = app.upgrade() {
                app.audio_device_changed(status);
            }
        });
        // The drum machine's state lives in the engine: keep it.
        self.capture_drums();
        self.engine.set(Some(handle));
        *self.audio_status.write().expect("audio status lock") = status.clone();
        *self.audio_runner.lock().expect("audio lock") = Some(runner);
        self.send_engine_settings();
        match (status.error, status.waiting_for) {
            (Some(err), Some(device)) => self.notify(UiEvent::Status(format!(
                "Audio output unavailable ({err}); running silent until {device} is connected"
            ))),
            (Some(err), None) => {
                self.notify(UiEvent::Status(format!("Audio output unavailable ({err}); running silent")))
            }
            (None, _) => {}
        }
        // A new engine starts empty: reload whatever the decks held.
        for d in 0..MAX_DECKS as u8 {
            let info = self.deck(d);
            if info.remix.is_some() {
                self.install_remix(d);
            } else if let Some(id) = info.track_id {
                self.load_track(d, id);
            }
        }
        self.install_drums();
        self.notify(UiEvent::AudioChanged);
    }

    /// The audio device went away or came back. The engine (and what the
    /// decks play) carries on either way.
    fn audio_device_changed(&self, status: AudioStatus) {
        let message = match (&status.waiting_for, &status.error) {
            (Some(device), _) => format!("Audio output lost: {device}. The decks play on; waiting for it to come back"),
            (None, None) => format!("Audio output back: {}", status.device),
            (None, Some(e)) => format!("Audio output: {e}"),
        };
        *self.audio_status.write().expect("audio status lock") = status;
        self.send_engine_settings();
        self.notify(UiEvent::Status(message));
        self.notify(UiEvent::AudioChanged);
    }

    /// Whether the decks go to a hardware mixer's channels (see
    /// [`MixingMode`]) instead of the internal mixer.
    pub fn external_mixing(&self) -> bool {
        match self.settings().mixing {
            MixingMode::Auto => self.audio_status().external_mixer,
            MixingMode::Internal => false,
            MixingMode::External => true,
        }
    }

    fn send_engine_settings(&self) {
        let s = self.settings();
        self.send(Command::Settings(rille_engine::Settings {
            tempo_range: s.tempo_range,
            split_cue: s.split_cue,
            external_outputs: self.external_mixing().then(|| s.deck_outputs()),
            ..rille_engine::Settings::default()
        }));
    }

    // -------------------------------------------------------------- control

    /// A button, knob or fader from the UI or keyboard.
    pub fn control(&self, ev: ControlEvent) {
        if let Some(session) = self.learn.lock().expect("learn lock").as_mut()
            && session.target() != ev.target
        {
            // Learn mode: clicking a control on screen selects it as target.
            *session = LearnSession::new(ev.target);
            self.notify(UiEvent::MidiLearned(format!("Move a control for {}", ev.target)));
            return;
        }
        match ev.target.control {
            Control::LoadSelected
            | Control::BrowserScroll
            | Control::BrowserTreeScroll
            | Control::BrowserToggleNode => {
                self.notify(UiEvent::Browser(ev));
            }
            Control::Eject if ev.value.is_press() => self.eject(ev.target.unit),
            Control::Record => {
                if ev.value.is_press() {
                    self.toggle_recording();
                }
            }
            Control::DrumShow => {
                if ev.value.is_press() {
                    self.toggle_drums_visible();
                }
            }
            Control::DrumKitSelect => {
                if let (ControlValue::Delta(d), Some(app)) = (ev.value, self.me.upgrade())
                    && d.round() != 0.0
                {
                    app.step_drum_kit(d.round() as i64);
                }
            }
            Control::DrumKit(n) => {
                if let (true, Some(app)) = (ev.value.is_press(), self.me.upgrade()) {
                    app.pick_drum_kit(usize::from(n).saturating_sub(1));
                }
            }
            Control::DrumCopy if ev.value.is_press() => {
                self.drum_copy_pattern();
                self.notify(UiEvent::DrumsChanged);
            }
            Control::DrumPaste if ev.value.is_press() => self.drum_paste_pattern(),
            // Cell edits need the app (files, the other decks' audio).
            Control::RemixPadLoad(_) => self.notify(UiEvent::Browser(ev)),
            // The browser knows the selected track.
            Control::DrumLoadSelected(_) => self.notify(UiEvent::Browser(ev)),
            Control::RemixPadDelete(pad) | Control::RemixPadCapture(pad) | Control::RemixPadType(pad) => {
                let deck = ev.target.unit;
                if ev.value.is_press()
                    && let Some(cell) = self.remix_pad_cell(deck, pad)
                {
                    match ev.target.control {
                        Control::RemixPadDelete(_) => self.delete_remix_cell(deck, cell),
                        Control::RemixPadCapture(_) => self.capture_remix_cell(deck, cell),
                        _ => self.toggle_remix_cell_type(deck, cell),
                    }
                }
            }
            _ => self.send(Command::Control(ev)),
        }
    }

    /// Called about 60 times per second by the UI.
    pub fn tick(&self) {
        let now = Instant::now();
        let dt = {
            let mut last = self.last_tick.lock().expect("tick lock");
            let dt = now.duration_since(*last).as_secs_f64().min(0.5);
            *last = now;
            dt
        };
        self.watch_drives(now);
        if let Some(engine) = self.engine.get() {
            engine.poll(|e| self.on_engine_event(e));
            // Dropouts, at most one line a second.
            let mut logged = self.xruns_logged.lock().expect("xrun log lock");
            let xruns = engine.xruns();
            if xruns > logged.1 && now.duration_since(logged.0) >= Duration::from_secs(1) {
                eprintln!("audio: {} dropouts (buffer underruns), {xruns} in all", xruns - logged.1);
                *logged = (now, xruns);
            }
            drop(logged);
            let snap = engine.snapshot();
            self.log_history(&snap, dt);
            self.drums_tick(&snap.drums);
            if self.settings.read().expect("settings lock").suggestions {
                self.follow_playing_track(&snap);
            }
        }
        if let Some(m) = self.midi.lock().expect("midi lock").as_mut() {
            m.send_feedback(&values::SnapshotValues::frozen(&self.engine, self.app_values()));
        }
    }

    fn deck_by_engine_id(&self, engine_id: u64) -> Option<(u8, TrackId)> {
        (0..MAX_DECKS as u8).find_map(|d| {
            let info = self.deck(d);
            (info.engine_id == engine_id).then_some(info.track_id.map(|t| (d, t))).flatten()
        })
    }

    fn on_engine_event(&self, e: Event) {
        let update = |track_id: u64, f: &dyn Fn(&mut TrackCues)| {
            if let Some((_, id)) = self.deck_by_engine_id(track_id) {
                let mut lib = self.library.lock().expect("library lock");
                let mut cues = lib.cues(id).unwrap_or_default();
                f(&mut cues);
                if let Err(err) = lib.set_cues(id, &cues) {
                    eprintln!("saving cues: {err}");
                }
            }
        };
        match e {
            Event::MainCueSet { track_id, secs, .. } => update(track_id, &|c| c.main_cue_secs = secs),
            Event::HotcueSet { track_id, slot, cue, .. } => update(track_id, &|c| {
                c.set_hotcue(CuePoint {
                    slot: Some(slot),
                    kind: cue.kind,
                    start_secs: cue.secs,
                    len_secs: cue.len_secs,
                    name: String::new(),
                    color: HOTCUE_COLORS[usize::from(slot) % HOTCUE_COLORS.len()],
                })
            }),
            Event::HotcueDeleted { track_id, slot, .. } => update(track_id, &|c| c.delete_hotcue(slot)),
            Event::TrackEnded { .. } => {}
        }
    }

    /// Logs a track to the history once it played 30 s with its fader open.
    fn log_history(&self, snap: &Snapshot, dt: f64) {
        for d in 0..MAX_DECKS {
            let (ds, ch) = (&snap.decks[d], &snap.channels[d]);
            if !(ds.playing && ch.volume > 0.1) {
                continue;
            }
            let mut info = self.decks[d].write().expect("deck lock");
            info.play_secs += dt;
            if info.play_secs > 30.0 && !info.logged {
                info.logged = true;
                if let Some(id) = info.track_id {
                    drop(info);
                    let mut lib = self.library.lock().expect("library lock");
                    let mut session = self.history_session.lock().expect("history lock");
                    if session.is_none() {
                        *session = lib.start_history_session().ok();
                    }
                    if let Some(s) = *session {
                        let _ = lib.log_played(s, id, d as u8);
                    }
                    drop((session, lib));
                    self.played.write().expect("played lock").insert(id);
                    // The row's played mark, play count and last played.
                    self.refresh_track(id);
                    self.add_recorded_track(id, d);
                }
            }
        }
    }

    /// Puts a track that counts as played into the recording's cue sheet,
    /// from when it became audible.
    fn add_recorded_track(&self, id: TrackId, deck: usize) {
        let mut recording = self.recording.lock().expect("recording lock");
        let Some(rec) = recording.as_mut() else { return };
        let Some(row) = self.track_row(id) else { return };
        let played = self.decks[deck].read().expect("deck lock").play_secs;
        rec.add_track(rec.secs() - played, &row.artist, &row.title);
    }

    /// Starts recording the main mix, or stops the recording.
    pub fn toggle_recording(&self) {
        if self.recording_on.load(Ordering::Relaxed) {
            self.stop_recording();
        } else if let Err(e) = self.start_recording() {
            self.notify(UiEvent::Status(e));
        }
    }

    /// Records the main mix to a new WAV file (with a cue sheet of the tracks
    /// played) in the recordings folder; returns its path.
    pub fn start_recording(&self) -> Result<PathBuf, String> {
        let mut slot = self.recording.lock().expect("recording lock");
        if let Some(r) = slot.as_ref() {
            return Ok(r.path.clone());
        }
        if self.external_mixing() {
            return Err("Recording takes the internal mix; with external mixing, record on the mixer".into());
        }
        let engine = self.engine.get().ok_or("Recording needs the audio engine")?;
        let (rec, recorder) = record::Recording::start(&self.paths.recordings, self.audio_status().sample_rate)
            .map_err(|e| format!("Cannot start recording: {e}"))?;
        if engine.send(Command::Record(Some(recorder))).is_err() {
            return Err("Cannot start recording: the engine is busy".into());
        }
        let path = rec.path.clone();
        *slot = Some(rec);
        self.recording_on.store(true, Ordering::Relaxed);
        self.notify(UiEvent::Status(format!("Recording to {}", path.display())));
        Ok(path)
    }

    /// Stops the recording and saves the rest of it; `None` if none ran.
    pub fn stop_recording(&self) -> Option<record::Recorded> {
        let rec = self.recording.lock().expect("recording lock").take()?;
        self.recording_on.store(false, Ordering::Relaxed);
        self.send(Command::Record(None));
        match rec.finish() {
            Ok(done) => {
                let lost = if done.dropped > 0 { " (the disk was too slow: some audio is missing)" } else { "" };
                let (m, s) = ((done.secs / 60.0) as u64, (done.secs % 60.0) as u64);
                self.notify(UiEvent::Status(format!("Recorded {m}:{s:02} to {}{lost}", done.path.display())));
                Some(done)
            }
            Err(e) => {
                self.notify(UiEvent::Status(format!("Recording failed: {e}")));
                None
            }
        }
    }

    /// The running recording: its file and seconds recorded.
    pub fn recording(&self) -> Option<(PathBuf, f64)> {
        self.recording.lock().expect("recording lock").as_ref().map(|r| (r.path.clone(), r.secs()))
    }

    /// Points the suggestions at the track on air: the master deck while it
    /// plays with its fader open, else the deck that has played longest
    /// that way. With nothing audible they stay with the last track (or the
    /// first loaded deck before anything played). Loading or ejecting a
    /// track updates them too.
    fn follow_playing_track(&self, snap: &Snapshot) {
        let loaded = |d: usize| {
            let info = self.decks[d].read().expect("deck lock");
            info.track_id.filter(|_| info.remix.is_none()).map(|t| (t, info.play_secs))
        };
        let audible = |d: usize| snap.decks[d].playing && snap.channels[d].volume > 0.1;
        let playing: Vec<(usize, TrackId, f64)> =
            (0..MAX_DECKS).filter(|&d| audible(d)).filter_map(|d| loaded(d).map(|(t, secs)| (d, t, secs))).collect();
        let on_air = playing
            .iter()
            .find(|(d, ..)| snap.decks[*d].master)
            .or_else(|| playing.iter().max_by(|a, b| a.2.total_cmp(&b.2)))
            .map(|&(d, t, _)| (d as u8, t));
        let mut current = self.suggest_for.lock().expect("suggest lock");
        let next = SuggestFor {
            reference: on_air
                .or(current.reference)
                .or_else(|| (0..MAX_DECKS).find_map(|d| loaded(d).map(|(t, _)| (d as u8, t)))),
            loaded: std::array::from_fn(|d| self.decks[d].read().expect("deck lock").track_id),
        };
        if next != *current {
            *current = next;
            drop(current);
            self.notify(UiEvent::SuggestionsChanged);
        }
    }

    /// The deck and track the suggestions are for.
    pub fn suggestion_reference(&self) -> Option<(u8, TrackRow)> {
        let (deck, id) = self.suggest_for.lock().expect("suggest lock").reference?;
        Some((deck, self.track_row(id)?))
    }

    /// Suggested tracks for [`Self::suggestion_reference`], best first;
    /// empty while suggestions are switched off. Leaves out the tracks on
    /// the decks and those played in this session.
    pub fn suggestions(&self) -> Vec<Suggestion> {
        if !self.settings().suggestions {
            return Vec::new();
        }
        let Some((_, reference)) = self.suggestion_reference() else { return Vec::new() };
        let mut exclude: HashSet<TrackId> = (0..MAX_DECKS as u8).filter_map(|d| self.deck_track(d)).collect();
        exclude.extend(self.played.read().expect("played lock").iter().copied());
        suggest::suggest(&reference, &self.tracks.read().expect("tracks lock").rows, &exclude)
    }

    // --------------------------------------------------------------- decks

    /// Loads a library track onto a deck (decoding and, if needed, analysis
    /// run in the background; audio plays as soon as it is decoded).
    pub fn load_track(self: &Arc<Self>, deck: u8, id: TrackId) {
        if self.is_remix_deck(deck) {
            // Into the first free cell.
            self.load_remix_cell(deck, None, id);
            return;
        }
        if self.refuse_load(deck) {
            return;
        }
        let d = usize::from(deck).min(MAX_DECKS - 1);
        let row = match self.library.lock().expect("library lock").track(id) {
            Ok(Some(r)) => r,
            _ => return,
        };
        let seq = self.load_seq.fetch_add(1, Ordering::Relaxed);
        let previous = {
            let mut info = self.decks[d].write().expect("deck lock");
            let previous = info.track_id;
            let revision = info.revision + 1;
            *info = DeckInfo {
                track_id: Some(id),
                engine_id: seq,
                title: row.title.clone(),
                artist: row.artist.clone(),
                album: row.album.clone(),
                remixer: row.remixer.clone(),
                label: row.label.clone(),
                key: row.key,
                cover: self.library.lock().expect("library lock").cover_path(id, CoverSize::Large),
                path: Some(row.path.clone()),
                streamed: row.beatport_id.is_some(),
                loading: true,
                revision,
                ..DeckInfo::default()
            };
            previous
        };
        self.notify(UiEvent::DeckChanged(deck));
        // The rows' deck marks.
        self.notify(UiEvent::TracksChanged(previous.into_iter().chain([id]).collect()));
        let app = self.clone();
        std::thread::Builder::new()
            .name(format!("load-deck-{deck}"))
            .spawn(move || app.load_worker(deck, id, seq, row.path))
            .expect("spawn loader");
    }

    /// Whether the load lock keeps tracks off `deck`: it is protected,
    /// playing, its channel fader is at the lock level or above and the
    /// crossfader does not cut it. Remix decks are never locked (a load
    /// fills a free cell).
    pub fn load_locked(&self, deck: u8) -> bool {
        let s = self.settings();
        let letter = char::from(b'A' + deck.min(MAX_DECKS as u8 - 1));
        if !s.load_lock || !s.load_lock_decks.contains(letter) || self.is_remix_deck(deck) {
            return false;
        }
        let snap = self.snapshot();
        let d = usize::from(deck).min(MAX_DECKS - 1);
        snap.decks[d].playing && snap.channels[d].volume >= s.load_lock_level && snap.crossfader_gain(d) > 0.0
    }

    /// Tells the user and returns true when the load lock keeps tracks off
    /// `deck`.
    fn refuse_load(&self, deck: u8) -> bool {
        if !self.load_locked(deck) {
            return false;
        }
        let letter = char::from(b'A' + deck.min(MAX_DECKS as u8 - 1));
        self.notify(UiEvent::Status(format!(
            "Deck {letter} is on air: pull its fader down or stop it to load a track"
        )));
        true
    }

    /// Loads a file (drag and drop, file browser). A file outside the
    /// collection becomes a guest track: it keeps its analysis and cues but
    /// stays out of the collection until imported. Tags are read in the
    /// background, so a slow USB stick does not hold up the UI.
    pub fn load_file(self: &Arc<Self>, deck: u8, path: &Path) {
        if self.is_remix_deck(deck) {
            self.load_remix_file(deck, None, path);
            return;
        }
        if self.refuse_load(deck) {
            return;
        }
        let app = self.clone();
        let path = path.to_owned();
        std::thread::Builder::new()
            .name(format!("open-deck-{deck}"))
            .spawn(move || {
                let id = app.library.lock().expect("library lock").import_file_guest(&path);
                match id {
                    Ok(id) => {
                        app.refresh_track(id);
                        app.load_track(deck, id);
                    }
                    Err(e) => app.notify(UiEvent::Status(format!("Cannot load {}: {e}", path.display()))),
                }
            })
            .expect("spawn file loader");
    }

    pub fn eject(&self, deck: u8) {
        if self.is_remix_deck(deck) {
            self.clear_remix(deck);
            return;
        }
        self.send(Command::Unload { deck });
        let d = usize::from(deck).min(MAX_DECKS - 1);
        let mut info = self.decks[d].write().expect("deck lock");
        let previous = info.track_id;
        let revision = info.revision + 1;
        *info = DeckInfo { revision, ..DeckInfo::default() };
        drop(info);
        self.notify(UiEvent::DeckChanged(deck));
        if let Some(id) = previous {
            self.notify(UiEvent::TracksChanged(vec![id]));
        }
    }

    fn update_deck(&self, deck: u8, seq: u64, f: impl FnOnce(&mut DeckInfo)) -> bool {
        let d = usize::from(deck).min(MAX_DECKS - 1);
        let mut info = self.decks[d].write().expect("deck lock");
        if info.engine_id != seq {
            return false;
        }
        f(&mut info);
        info.revision += 1;
        drop(info);
        self.notify(UiEvent::DeckChanged(deck));
        true
    }

    fn load_worker(self: Arc<Self>, deck: u8, id: TrackId, seq: u64, path: PathBuf) {
        // What the library knows already: the analysis, cues and gain.
        let (stored, waveform, cues) = {
            let lib = self.library.lock().expect("library lock");
            (lib.analysis(id).ok().flatten(), lib.waveform(id).ok().flatten(), lib.cues(id).unwrap_or_default())
        };
        let fresh = stored.as_ref().is_some_and(|a| a.analyzer_version >= ANALYZER_VERSION) && waveform.is_some();
        let settings = self.settings();
        let auto_gain = if settings.auto_gain {
            stored.as_ref().and_then(|a| a.lufs).map_or(0.0, |l| (settings.target_lufs - l).clamp(-12.0, 12.0))
        } else {
            0.0
        };
        let grid = stored.as_ref().and_then(|a| a.grid.clone()).map(Arc::new);
        let load = |audio: Arc<TrackAudio>| {
            self.send(Command::Load {
                deck,
                track: LoadedTrack {
                    id: seq,
                    audio,
                    grid: if fresh { grid.clone() } else { None },
                    main_cue_secs: cues.main_cue_secs,
                    hotcues: hotcues_from(&cues),
                    auto_gain_db: auto_gain,
                },
            });
        };

        // A streamed track whose file is not in the cache downloads first;
        // it starts playing while the rest comes.
        let row = self.library.lock().expect("library lock").track(id).ok().flatten();
        let audio = match row.filter(|r| r.beatport_id.is_some()) {
            Some(row) => match self.stream_to_deck(deck, seq, &row, &load) {
                Some(Streamed::Decoded(audio, file)) => {
                    // The downloaded file may bring the cover the catalog had not.
                    let cover = self.library.lock().expect("library lock").cover_path(id, CoverSize::Large);
                    self.update_deck(deck, seq, |i| {
                        i.path = Some(file);
                        i.cover = i.cover.take().or(cover);
                    });
                    audio
                }
                Some(Streamed::Failed(e)) => {
                    if self.update_deck(deck, seq, |i| {
                        i.loading = false;
                        i.error = Some(e.clone());
                    }) {
                        self.notify(UiEvent::Status(format!("Cannot stream {}: {e}", row.title)));
                    }
                    return;
                }
                None => return,
            },
            None => match rille_decode::decode_file(&path, None, &mut |_| {}) {
                Ok(audio) => audio,
                Err(e) => {
                    self.update_deck(deck, seq, |i| {
                        i.loading = false;
                        i.error = Some(e.to_string());
                    });
                    self.notify(UiEvent::Status(format!("Cannot decode {}: {e}", path.display())));
                    return;
                }
            },
        };
        if self.deck(deck).engine_id != seq {
            return;
        }
        let track_audio = Arc::new(TrackAudio { sample_rate: audio.sample_rate, frames: audio.frames.clone() });
        let streamed = self.deck(deck).audio.is_some();
        if !self.update_deck(deck, seq, |i| i.audio = Some(track_audio.clone())) {
            return;
        }
        if streamed {
            // Already playing the first part: now all of it.
            let length_secs = None;
            self.send(Command::ExtendTrack { deck, track_id: seq, audio: track_audio.clone(), length_secs });
        } else {
            load(track_audio.clone());
        }
        // Stems separated before come along.
        self.load_cached_stems(deck, seq, id, &track_audio);
        if fresh {
            let wf = waveform.and_then(|b| WaveformSummary::from_bytes(&b)).map(Arc::new);
            self.update_deck(deck, seq, |i| {
                i.loading = false;
                i.grid = grid;
                i.waveform = wf;
                i.key = stored.as_ref().and_then(|a| a.key).or(i.key);
            });
            return;
        }

        // Not analyzed yet: the deck already plays; the grid follows.
        self.update_deck(deck, seq, |i| {
            i.loading = false;
            i.analyzing = true;
        });
        // Shown as running in the browser; a worker won't pick it up too.
        self.queue.claim(id);
        self.notify_progress();
        let out = rille_analysis::analyze(&audio, &analysis_config(&settings));
        let effective = {
            let mut lib = self.library.lock().expect("library lock");
            let _ = lib.set_analysis(id, &out.analysis);
            let _ = lib.set_waveform(id, &out.waveform.to_bytes());
            lib.analysis(id).ok().flatten()
        };
        self.finish_analysis(id, true);
        self.notify_progress();
        let grid = effective.as_ref().and_then(|a| a.grid.clone()).map(Arc::new);
        self.send(Command::SetGrid { deck, track_id: seq, grid: grid.clone() });
        let wf = Arc::new(out.waveform);
        self.update_deck(deck, seq, |i| {
            i.analyzing = false;
            i.grid = grid;
            i.waveform = Some(wf);
            i.key = out.analysis.key.or(i.key);
        });
        self.refresh_track(id);
    }

    /// A streamed track for a deck: from the cache, or downloaded. While it
    /// downloads, the first seconds are decoded and `load`ed as soon as they
    /// are there, and the deck gets longer copies as more arrives
    /// ([`Command::ExtendTrack`]); the caller sends the whole track. `None`
    /// when the deck moved on to another track.
    fn stream_to_deck(
        self: &Arc<Self>,
        deck: u8,
        seq: u64,
        row: &TrackRow,
        load: &dyn Fn(Arc<TrackAudio>),
    ) -> Option<Streamed> {
        let decode = |path: &Path| match rille_decode::decode_file(path, None, &mut |_| {}) {
            Ok(audio) => Streamed::Decoded(audio, path.to_owned()),
            Err(e) => Streamed::Failed(e.to_string()),
        };
        if row.path.exists() {
            return Some(match self.streamed_file(row, Some((deck, seq)), None) {
                Ok(p) => decode(&p),
                Err(e) => Streamed::Failed(e),
            });
        }
        let arrived = rille_beatport::Arrived::new();
        let download = {
            let (app, row, arrived) = (self.clone(), row.clone(), arrived.clone());
            std::thread::Builder::new()
                .name(format!("stream-deck-{deck}"))
                .spawn(move || app.streamed_file(&row, Some((deck, seq)), Some(&arrived)))
                .expect("spawn stream download")
        };
        // Wait for the first bytes, or for the download to end (it was in
        // the cache after all, another download had it, or it failed).
        let started = loop {
            if let Some(s) = arrived.started(Duration::from_millis(100)) {
                break Some(s);
            }
            // Ended without starting: `started` no longer waits, so stop
            // polling while the download thread winds up (it is joined).
            if download.is_finished() || arrived.outcome().is_some() {
                break None;
            }
        };
        let finished = |download: std::thread::JoinHandle<Result<PathBuf, String>>| {
            download.join().unwrap_or_else(|_| Err("the download crashed".into()))
        };
        let Some((_, total)) = started else {
            return Some(finished(download).map_or_else(Streamed::Failed, |p| decode(&p)));
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let reader = match arrived.reader(cancel.clone()) {
            Ok(r) => r,
            // Gone already: it finished and was renamed.
            Err(_) => return Some(finished(download).map_or_else(Streamed::Failed, |p| decode(&p))),
        };
        // A hint only: the file type is probed from its first bytes.
        let ext = row.path.extension().and_then(|e| e.to_str());
        let (mut next_push, mut pushed, mut next_check) = (0usize, 0usize, 0usize);
        let mut on_frames = |so_far: &rille_decode::DecodedAudio| {
            let sr = so_far.sample_rate as usize;
            if self.deck(deck).engine_id != seq {
                cancel.store(true, Ordering::Relaxed);
                return;
            }
            let have = so_far.frames.len();
            if sr == 0 {
                return;
            }
            // Longer copies at doubling lengths: about twice the track copied
            // in all. When the download is slow, playback can reach the end
            // of the copy the deck has while much more is decoded: then the
            // decoded audio goes out at once (checked once per second of it).
            let due = have >= next_push.max(STREAM_START_SECS * sr)
                || pushed > 0 && have >= pushed + STREAM_EARLY_SECS * sr && have >= next_check && {
                    next_check = have + sr;
                    let d = self.snapshot().decks[deck as usize];
                    d.buffering || (d.position_secs + STREAM_NEAR_END_SECS) * sr as f64 >= pushed as f64
                };
            if !due {
                return;
            }
            next_push = 2 * have;
            pushed = have;
            let audio = Arc::new(TrackAudio { sample_rate: so_far.sample_rate, frames: so_far.frames.clone() });
            let secs = audio.duration_secs();
            // The whole length from the catalog, else estimated from the bytes.
            let arrived_bytes = arrived.contiguous().max(1) as f64;
            let length = if row.duration_secs > secs { row.duration_secs } else { secs * total as f64 / arrived_bytes };
            let first = self.deck(deck).audio.is_none();
            if !self.update_deck(deck, seq, |i| {
                i.audio = Some(audio.clone());
                i.loading = false;
            }) {
                return;
            }
            if first {
                load(audio.clone());
            }
            let length_secs = Some(length.max(secs));
            self.send(Command::ExtendTrack { deck, track_id: seq, audio, length_secs });
        };
        let decoded = rille_decode::decode_reader(reader, ext, None, &mut on_frames);
        let file = finished(download);
        if self.deck(deck).engine_id != seq {
            return None;
        }
        match (decoded, file) {
            (Ok(audio), Ok(file)) => Some(Streamed::Decoded(audio, file)),
            (_, Err(e)) | (Err(rille_decode::DecodeError::Unsupported(e)), _) => {
                // What already plays ends where the download stopped.
                if let Some(audio) = self.deck(deck).audio {
                    self.send(Command::ExtendTrack { deck, track_id: seq, audio, length_secs: None });
                }
                Some(Streamed::Failed(e))
            }
            (Err(e), Ok(_)) => Some(Streamed::Failed(e.to_string())),
        }
    }

    /// Applies a beatgrid correction to the track on `deck` and saves it.
    pub fn grid_edit(&self, deck: u8, edit: GridEdit) {
        let d = usize::from(deck).min(MAX_DECKS - 1);
        let info = self.deck(deck);
        let Some(id) = info.track_id else { return };
        let pos = self.snapshot().decks[d].position_secs;
        let new = match (edit, info.grid.as_deref()) {
            (GridEdit::Reset, _) => {
                let mut lib = self.library.lock().expect("library lock");
                let _ = lib.reset_grid(id);
                lib.analysis(id).ok().flatten().and_then(|a| a.grid)
            }
            (GridEdit::Tap, grid) => {
                let mut taps = {
                    let mut w = self.decks[d].write().expect("deck lock");
                    let now = Instant::now();
                    if w.taps.last().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(2)) {
                        w.taps.clear();
                    }
                    w.taps.push(now);
                    if w.taps.len() > 16 {
                        w.taps.remove(0);
                    }
                    w.taps.clone()
                };
                if taps.len() < 4 {
                    return;
                }
                let t0 = taps[0];
                let secs: Vec<f64> = taps.drain(..).map(|t| t.duration_since(t0).as_secs_f64()).collect();
                let Some(tapped) = BeatGrid::from_taps(&secs) else { return };
                let bpm = tapped.bpm_at(0.0);
                // Keep the grid position under the play head, set the tempo.
                let anchor = grid.map_or(pos, |g| g.secs_at(g.beat_at(pos).round()));
                let Ok(map) = rille_core::BeatMap::constant(anchor, bpm) else { return };
                let mut g = BeatGrid::new(map, rille_core::GridSource::Tapped);
                if let Some(old) = grid {
                    g.downbeat_beat_index = old.beat_at(old.secs_at(old.downbeat_beat_index as f64)).round() as i64;
                }
                Some(g)
            }
            (_, None) => None,
            (GridEdit::Move(ms), Some(g)) => Some(g.shifted(ms / 1000.0)),
            (GridEdit::DoubleTempo, Some(g)) => Some(g.scaled(2.0, pos)),
            (GridEdit::HalveTempo, Some(g)) => Some(g.scaled(0.5, pos)),
            (GridEdit::BeatHere, Some(g)) => Some(g.with_beat_at(pos)),
            (GridEdit::DownbeatHere, Some(g)) => Some(g.with_downbeat_at(pos)),
            (GridEdit::BarStartHere, Some(g)) => {
                let at = info.audio.as_deref().and_then(|a| attack_near(a, pos)).unwrap_or(pos);
                Some(g.with_bar_start_at(at))
            }
            (GridEdit::Lock(on), Some(g)) => {
                let mut g = g.clone();
                g.locked = on;
                Some(g)
            }
        };
        let Some(new) = new else { return };
        if !matches!(edit, GridEdit::Reset) {
            let _ = self.library.lock().expect("library lock").set_grid(id, &new);
        }
        let grid = Arc::new(new);
        self.send(Command::SetGrid { deck, track_id: info.engine_id, grid: Some(grid.clone()) });
        self.update_deck(deck, info.engine_id, |i| i.grid = Some(grid));
        self.refresh_tracks();
    }

    // ------------------------------------------------------------- library

    /// Rows for the browser.
    pub fn tracks(&self, source: Source, search: &str, sort: SortKey, descending: bool) -> Vec<TrackRow> {
        // Before taking the tracks lock: suggestions read it too, and the
        // library is never locked while the tracks lock is held.
        let ids: Option<Vec<TrackId>> = match source {
            Source::Collection => None,
            Source::Playlist(p) => self.library.lock().expect("library lock").playlist_tracks(p).ok(),
            Source::History(h) => self.library.lock().expect("library lock").history_tracks(h).ok(),
            Source::Suggestions => Some(self.suggestions().into_iter().map(|s| s.id).collect()),
            Source::BeatportRecent => self.library.lock().expect("library lock").streamed_tracks(false).ok(),
            Source::BeatportOffline => self.library.lock().expect("library lock").streamed_tracks(true).ok(),
        };
        let t = self.tracks.read().expect("tracks lock");
        let matching: Option<HashSet<usize>> =
            (!search.trim().is_empty()).then(|| t.index.search(search).into_iter().collect());
        let by_id = |id: TrackId| t.by_id.get(&id).copied();
        let mut rows: Vec<TrackRow> = match ids {
            // Playlists, history and suggestions keep their own order unless sorted.
            Some(ids) => ids
                .into_iter()
                .filter_map(by_id)
                .filter(|i| matching.as_ref().is_none_or(|m| m.contains(i)))
                .map(|i| t.rows[i].clone())
                .collect(),
            // The collection is the local files; streamed tracks have their own
            // list, guest tracks show only in their folder.
            None => t
                .rows
                .iter()
                .enumerate()
                .filter(|(i, r)| r.beatport_id.is_none() && !r.guest && matching.as_ref().is_none_or(|m| m.contains(i)))
                .map(|(_, r)| r.clone())
                .collect(),
        };
        let ordered = matches!(
            source,
            Source::Playlist(_)
                | Source::History(_)
                | Source::Suggestions
                | Source::BeatportRecent
                | Source::BeatportOffline
        );
        if !ordered || descending || sort != SortKey::Artist {
            sort_rows(&mut rows, sort, descending);
        }
        rows
    }

    pub fn playlists(&self) -> Vec<PlaylistNode> {
        self.library.lock().expect("library lock").playlist_tree().unwrap_or_default()
    }

    pub fn history_sessions(&self) -> Vec<HistorySession> {
        self.library.lock().expect("library lock").history_sessions().unwrap_or_default()
    }

    pub fn create_playlist(&self, name: &str) -> Option<i64> {
        let id = self.library.lock().expect("library lock").create_playlist(None, name, false).ok();
        self.notify(UiEvent::LibraryChanged);
        id
    }

    pub fn add_to_playlist(&self, playlist: i64, tracks: &[TrackId]) {
        let _ = self.library.lock().expect("library lock").add_to_playlist(playlist, tracks, None);
        self.notify(UiEvent::LibraryChanged);
    }

    pub fn delete_playlist(&self, playlist: i64) {
        let _ = self.library.lock().expect("library lock").delete_playlist(playlist);
        self.notify(UiEvent::LibraryChanged);
    }

    pub fn set_rating(&self, id: TrackId, stars: u8) {
        let _ = self.library.lock().expect("library lock").set_rating(id, stars.min(5));
        self.refresh_track(id);
    }

    pub fn cover(&self, id: TrackId) -> Option<PathBuf> {
        self.library.lock().expect("library lock").cover_path(id, CoverSize::Small)
    }

    /// Adds a music folder and scans it in the background.
    pub fn add_music_folder(self: &Arc<Self>, dir: PathBuf) {
        let mut s = self.settings();
        if !s.library_roots.contains(&dir) {
            s.library_roots.push(dir);
            self.set_settings(s);
        }
        self.scan();
    }

    fn sync_roots(&self) {
        let mut lib = self.library.lock().expect("library lock");
        let wanted = self.settings().library_roots;
        let existing = lib.roots().unwrap_or_default();
        for r in &existing {
            if !wanted.contains(r) {
                let _ = lib.remove_root(r);
            }
        }
        for r in &wanted {
            if !existing.contains(r) {
                let _ = lib.add_root(r);
            }
        }
    }

    /// Rescans the music folders in the background. The scan works on its
    /// own database connection, so browsing stays responsive meanwhile.
    pub fn scan(self: &Arc<Self>) {
        self.sync_roots();
        let app = self.clone();
        std::thread::Builder::new()
            .name("scan".into())
            .spawn(move || {
                app.notify(UiEvent::Status("Scanning music folders…".into()));
                let mut clock = ReadClock::new();
                let report = app
                    .open_library()
                    .and_then(|mut lib| lib.scan(&mut |p| app.notify_scan(p, &mut clock)).map_err(|e| e.to_string()));
                match report {
                    Ok(r) => app.notify(UiEvent::Status(format!(
                        "Scan: {} new, {} updated, {} moved, {} missing · took {}",
                        r.added,
                        r.updated,
                        r.relinked,
                        r.missing,
                        format_duration(clock.started.elapsed())
                    ))),
                    Err(e) => app.notify(UiEvent::Status(format!("Scan failed: {e}"))),
                }
                app.notify(UiEvent::ImportProgress { done: 0, total: 0, timing: Timing::default() });
                app.refresh_tracks();
                let _ = app.analysis_kick.send(());
            })
            .expect("spawn scan");
    }

    /// A second connection to the collection for long background work.
    fn open_library(&self) -> Result<Library, String> {
        let lib = Library::open(&self.paths.library_db(), &self.paths.cache).map_err(|e| e.to_string())?;
        lib.set_busy_timeout(Duration::from_secs(10)).map_err(|e| e.to_string())?;
        Ok(lib)
    }

    fn notify_scan(&self, p: ScanProgress, clock: &mut ReadClock) {
        if let ScanProgress::Reading { done, total } = p {
            self.notify(UiEvent::ImportProgress { done, total, timing: clock.timing(done, total) });
        }
    }

    /// Adds files to the collection in the background (file explorer,
    /// drag and drop), then analyzes them if asked.
    pub fn import_paths(self: &Arc<Self>, paths: Vec<PathBuf>, analyze: bool) {
        self.import_in_background(paths, analyze, None);
    }

    /// Imports `paths`, then (with `playlists`) mirrors that folder tree
    /// as playlists.
    fn import_in_background(self: &Arc<Self>, paths: Vec<PathBuf>, analyze: bool, playlists: Option<FolderTree>) {
        let app = self.clone();
        std::thread::Builder::new()
            .name("import".into())
            .spawn(move || {
                let mut clock = ReadClock::new();
                let report = app.open_library().and_then(|mut lib| {
                    let r =
                        lib.import_files(&paths, &mut |p| app.notify_scan(p, &mut clock)).map_err(|e| e.to_string())?;
                    let mirrored = match &playlists {
                        Some(tree) => Some(mirror_folder(&mut lib, tree, None).map_err(|e| e.to_string())?),
                        None => None,
                    };
                    Ok((r, mirrored))
                });
                app.notify(UiEvent::ImportProgress { done: 0, total: 0, timing: Timing::default() });
                match report {
                    Ok((r, mirrored)) => {
                        let lists = match (&playlists, mirrored) {
                            (Some(tree), Some(n)) => format!(" · playlist \"{}\" (+{n} entries)", tree.name),
                            _ => String::new(),
                        };
                        app.notify(UiEvent::Status(format!(
                            "Import: {} new, {} updated, {} already in the collection{}{lists} · took {}",
                            r.added,
                            r.updated + r.relinked,
                            r.unchanged,
                            if r.errors.is_empty() {
                                String::new()
                            } else {
                                format!(", {} unreadable", r.errors.len())
                            },
                            format_duration(clock.started.elapsed())
                        )));
                        if playlists.is_some() {
                            app.notify(UiEvent::LibraryChanged);
                        }
                        app.refresh_tracks();
                        if analyze {
                            app.analyze_tracks(&r.ids, false);
                        }
                    }
                    Err(e) => app.notify(UiEvent::Status(format!("Import failed: {e}"))),
                }
            })
            .expect("spawn import");
    }

    /// Imports the audio files in `dir` (and below with `recursive`).
    pub fn import_folder(self: &Arc<Self>, dir: &Path, recursive: bool, analyze: bool) {
        let files = explorer::audio_files(dir, recursive);
        if files.is_empty() {
            self.notify(UiEvent::Status(format!("No audio files in {}", dir.display())));
            return;
        }
        self.import_paths(files, analyze);
    }

    /// Imports a folder and mirrors it as a playlist of the same name. With
    /// `recursive`, subfolders become a playlist folder of that name, each
    /// holding its own playlists. Importing again reuses the playlists and
    /// only adds files that are new.
    pub fn import_folder_as_playlist(self: &Arc<Self>, dir: &Path, recursive: bool, analyze: bool) {
        let tree = explorer::folder_tree(dir, recursive);
        if tree.is_empty() {
            self.notify(UiEvent::Status(format!("No audio files in {}", dir.display())));
            return;
        }
        self.import_in_background(tree.all_files(), analyze, Some(tree));
    }

    /// Queues collection tracks for analysis ahead of the background pass.
    /// `force` re-analyzes tracks that already have a current result.
    pub fn analyze_tracks(&self, ids: &[TrackId], force: bool) {
        let wanted: Vec<TrackId> = if force {
            ids.to_vec()
        } else {
            let t = self.tracks.read().expect("tracks lock");
            ids.iter()
                .copied()
                .filter(|id| {
                    t.by_id.get(id).is_none_or(|&i| t.rows[i].analysis_version.is_none_or(|v| v < ANALYZER_VERSION))
                })
                .collect()
        };
        // Tracks already waiting move up to the user's lane.
        self.queue.push(&wanted, Priority::User, force);
        self.notify(UiEvent::TracksChanged(wanted));
        self.notify_progress();
    }

    /// Analyzes files by path: imports the ones not yet in the collection.
    pub fn analyze_paths(self: &Arc<Self>, paths: Vec<PathBuf>, force: bool) {
        let mut known = Vec::new();
        let mut unknown = Vec::new();
        {
            let lib = self.library.lock().expect("library lock");
            for p in paths {
                match lib.track_by_path(&p) {
                    Ok(Some(id)) => known.push(id),
                    _ => unknown.push(p),
                }
            }
        }
        self.analyze_tracks(&known, force);
        if !unknown.is_empty() {
            self.import_paths(unknown, true);
        }
    }

    /// Drops the waiting jobs and turns the background pass off, so they
    /// don't come back until the user turns it on again.
    pub fn cancel_analysis(self: &Arc<Self>) {
        // Off before clearing, so the planner can't refill the queue.
        let mut s = self.settings();
        let was_on = s.background_analysis;
        s.background_analysis = false;
        s.analysis_paused = false;
        self.set_settings(s);
        self.queue.cancel();
        self.queue.set_paused(false);
        if was_on {
            self.notify(UiEvent::Status(
                "Analysis cancelled; background analysis is off (Settings → Analyze in the background)".into(),
            ));
        }
        self.notify_progress();
    }

    /// Pauses or resumes the queue; a pause lasts over restarts.
    pub fn set_analysis_paused(self: &Arc<Self>, paused: bool) {
        self.queue.set_paused(paused);
        let mut s = self.settings();
        s.analysis_paused = paused;
        self.set_settings(s);
        self.notify_progress();
    }

    pub fn analysis_progress(&self) -> AnalysisProgress {
        let p = self.queue.progress();
        let current = p
            .running
            .first()
            .map(|&id| {
                let t = self.tracks.read().expect("tracks lock");
                t.by_id.get(&id).map_or_else(String::new, |&i| {
                    let r = &t.rows[i];
                    match (r.artist.is_empty(), r.title.is_empty()) {
                        (false, false) => format!("{} – {}", r.artist, r.title),
                        (true, false) => r.title.clone(),
                        _ => r.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
                    }
                })
            })
            .unwrap_or_default();
        AnalysisProgress { done: p.done, failed: p.failed, total: p.total, paused: p.paused, current, timing: p.timing }
    }

    /// Elapsed and remaining analysis time, cheap enough for every frame.
    pub fn analysis_timing(&self) -> Timing {
        self.queue.timing()
    }

    fn notify_progress(&self) {
        self.notify(UiEvent::AnalysisProgress(self.analysis_progress()));
    }

    /// Ends an analysis job; the last one of a batch reports how long the
    /// batch took (a single deck load is not worth a message).
    fn finish_analysis(&self, id: TrackId, ok: bool) {
        if let Some(b) = self.queue.finish(id, ok)
            && b.done + b.failed > 1
        {
            let failed = if b.failed > 0 { format!(", {} failed", b.failed) } else { String::new() };
            self.notify(UiEvent::Status(format!(
                "Analyzed {} tracks in {}{failed}",
                b.done,
                format_duration(b.elapsed)
            )));
        }
    }

    /// Where a collection track stands in analysis.
    pub fn analysis_state(&self, row: &TrackRow) -> AnalysisState {
        if row.id < 0 {
            return AnalysisState::NotInCollection;
        }
        if let Some(s) = self.queue.state(row.id) {
            return s;
        }
        match row.analysis_version {
            _ if row.analysis_failed => AnalysisState::Failed,
            Some(v) if v >= ANALYZER_VERSION => AnalysisState::Done,
            Some(_) => AnalysisState::Stale,
            None => AnalysisState::NotAnalyzed,
        }
    }

    /// Number of tracks in the collection (local files, not streamed or
    /// guest ones).
    pub fn track_count(&self) -> usize {
        self.tracks.read().expect("tracks lock").rows.iter().filter(|r| r.beatport_id.is_none() && !r.guest).count()
    }

    /// Number of tracks streamed from Beatport.
    pub fn streamed_count(&self) -> usize {
        self.tracks.read().expect("tracks lock").rows.iter().filter(|r| r.beatport_id.is_some()).count()
    }

    /// Back to the analyzer's grid for these tracks (edits and imported
    /// grids dropped).
    pub fn reset_grids(&self, ids: &[TrackId]) {
        {
            let mut lib = self.library.lock().expect("library lock");
            for &id in ids {
                let _ = lib.reset_grid(id);
            }
        }
        for &id in ids {
            self.refresh_track_row(id);
        }
        self.notify(UiEvent::TracksChanged(ids.to_vec()));
    }

    /// Removes tracks from the collection (files stay on disk).
    pub fn remove_tracks(&self, ids: &[TrackId]) {
        let _ = self.library.lock().expect("library lock").remove_tracks(ids);
        self.refresh_tracks();
    }

    /// Sets (or clears) the color tag of tracks.
    pub fn set_track_color(&self, ids: &[TrackId], color: Option<u32>) {
        {
            let mut lib = self.library.lock().expect("library lock");
            for &id in ids {
                let _ = lib.set_color(id, color);
            }
        }
        for &id in ids {
            self.refresh_track_row(id);
        }
        self.notify(UiEvent::TracksChanged(ids.to_vec()));
    }

    /// The explorer's view of a folder: its audio files, with collection
    /// (or guest) data where the file has a track, else the tags read
    /// while browsing. Files whose tags are not read yet get a title from
    /// the file name (id −1) and are read in the background; a
    /// [`UiEvent::FolderMetaChanged`] follows.
    pub fn folder_rows(self: &Arc<Self>, dir: &Path) -> Vec<TrackRow> {
        let files = explorer::audio_files(dir, false);
        let canon_dir = dir.canonicalize().unwrap_or_else(|_| dir.to_owned());
        let canon: Vec<PathBuf> = files.iter().map(|p| p.canonicalize().unwrap_or_else(|_| p.clone())).collect();
        let (known, cached) = {
            let lib = self.library.lock().expect("library lock");
            let known: HashMap<PathBuf, TrackRow> = lib
                .tracks_under(&canon_dir, false)
                .unwrap_or_default()
                .into_iter()
                .map(|r| (r.path.clone(), r))
                .collect();
            let unknown: Vec<PathBuf> = canon.iter().filter(|p| !known.contains_key(*p)).cloned().collect();
            let cached = lib.file_meta(&unknown).unwrap_or_default();
            (known, cached)
        };
        let mut unread = Vec::new();
        let rows = files
            .iter()
            .zip(&canon)
            .map(|(p, c)| {
                if let Some(r) = known.get(c) {
                    return r.clone();
                }
                if let Some(m) = cached.get(c) {
                    let mut row = m.to_row();
                    row.path.clone_from(p);
                    return row;
                }
                unread.push(c.clone());
                explorer::file_row(p)
            })
            .collect();
        if !unread.is_empty() {
            self.request_folder_meta(canon_dir, unread);
        }
        rows
    }

    /// Cover thumbnail for a row's cover key, without touching the database.
    pub fn cover_file(&self, cover: &str, size: CoverSize) -> Option<PathBuf> {
        Some(rille_library::cover_thumb(&self.paths.cache, cover, size)).filter(|p| p.exists())
    }

    /// Imports a Traktor `collection.nml` (grids, cues, ratings).
    pub fn import_nml(&self, path: &Path) -> Result<String, String> {
        let report = self
            .library
            .lock()
            .expect("library lock")
            .import_nml(path, &|p| Some(PathBuf::from(p)))
            .map_err(|e| e.to_string())?;
        self.refresh_tracks();
        Ok(format!(
            "{} entries, {} matched, {} imported, {} grids, {} cues, {} missing",
            report.entries,
            report.matched,
            report.imported,
            report.grids,
            report.cues,
            report.missing.len()
        ))
    }

    fn refresh_tracks(&self) {
        // A separate statement, so the library lock is released before the
        // tracks lock is taken: holding both invites a deadlock with any
        // reader that takes them the other way round.
        let rows = self.library.lock().expect("library lock").tracks();
        if let Ok(rows) = rows {
            *self.tracks.write().expect("tracks lock") = Tracks::new(rows);
        }
        self.notify(UiEvent::LibraryChanged);
    }

    /// Re-reads one row (analysis finished, rating, color) and tells the UI
    /// which row changed instead of rebuilding every list.
    fn refresh_track(&self, id: TrackId) {
        self.refresh_track_row(id);
        self.notify(UiEvent::TracksChanged(vec![id]));
    }

    fn refresh_track_row(&self, id: TrackId) {
        let row = self.library.lock().expect("library lock").track(id).ok().flatten();
        let mut t = self.tracks.write().expect("tracks lock");
        match (row, t.by_id.get(&id).copied()) {
            (Some(row), Some(i)) => t.rows[i] = row,
            (Some(row), None) => {
                // New track: rebuild (search index, positions).
                let mut rows = std::mem::take(&mut t.rows);
                rows.push(row);
                *t = Tracks::new(rows);
            }
            _ => {}
        }
    }

    /// One collection row by id.
    pub fn track_row(&self, id: TrackId) -> Option<TrackRow> {
        let t = self.tracks.read().expect("tracks lock");
        t.by_id.get(&id).map(|&i| t.rows[i].clone())
    }

    // ------------------------------------------------------------ analysis

    fn spawn_analysis_workers(self: Arc<Self>, kick: Receiver<()>) {
        let workers = std::thread::available_parallelism().map_or(2, |n| (n.get() / 4).clamp(1, 4));
        self.queue.set_workers(workers);
        for w in 0..workers {
            let app = self.clone();
            std::thread::Builder::new()
                .name(format!("analysis-{w}"))
                .spawn(move || {
                    // Background work: never compete with playback or the UI.
                    rille_engine::realtime::lower_current_thread();
                    while let Some(job) = app.queue.take() {
                        if app.shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        app.notify_progress();
                        let ok = app.analyze_track(job.id);
                        app.finish_analysis(job.id, ok);
                        app.refresh_track(job.id);
                        app.notify_progress();
                    }
                })
                .expect("spawn analysis worker");
        }
        // The background pass: whatever in the collection lacks a current
        // analysis joins the queue's lowest lane, at start, after scans and
        // imports, and once a minute.
        let app = self;
        std::thread::Builder::new()
            .name("analysis-planner".into())
            .spawn(move || {
                loop {
                    let _ = kick.recv_timeout(Duration::from_secs(60));
                    while kick.try_recv().is_ok() {}
                    if app.shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    if !app.settings().background_analysis {
                        continue;
                    }
                    let ids = app.library.lock().expect("library lock").tracks_needing_analysis().unwrap_or_default();
                    if app.queue.push(&ids, Priority::Background, false) > 0 {
                        app.notify_progress();
                    }
                }
            })
            .expect("spawn analysis planner");
    }

    /// Analyzes one collection track and stores the result; a failure is
    /// recorded so the file is not retried until the analyzer changes.
    fn analyze_track(&self, id: TrackId) -> bool {
        let row = self.library.lock().expect("library lock").track(id).ok().flatten();
        let Some(row) = row else { return false };
        // A streamed track out of the cache is analyzed when it is loaded
        // again; that is no failure of the analyzer.
        if row.beatport_id.is_some() && !row.path.exists() {
            return false;
        }
        let path = row.path;
        let cfg = analysis_config(&self.settings());
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rille_analysis::analyze_file(&path, &cfg, None)));
        let mut lib = self.library.lock().expect("library lock");
        match result {
            Ok(Ok((_, out))) => {
                let _ = lib.set_analysis(id, &out.analysis);
                let _ = lib.set_waveform(id, &out.waveform.to_bytes());
                true
            }
            Ok(Err(e)) => {
                let _ = lib.set_analysis_error(id, &e.to_string());
                false
            }
            Err(_) => {
                let _ = lib.set_analysis_error(id, "analysis crashed");
                false
            }
        }
    }

    // ----------------------------------------------------------------- MIDI

    fn start_midi(self: &Arc<Self>) {
        let (tx, rx) = crossbeam_channel::unbounded::<MidiEvent>();
        self.refresh_drum_kit_list();
        let values = Arc::new(values::SnapshotValues::live(self.engine.clone(), self.app_values()));
        match MidiManager::new("rille", tx, values) {
            Ok(mut m) => {
                let store = self.mappings.lock().expect("mappings lock");
                m.set_hid_layouts(store.hid_layouts());
                m.refresh_with(|port, connected| self.pick_mapping(&store, port, connected));
                drop(store);
                *self.midi.lock().expect("midi lock") = Some(m);
            }
            Err(e) => {
                self.notify(UiEvent::Status(format!("MIDI unavailable: {e:?}")));
                return;
            }
        }
        // MIDI input goes straight to the engine (lowest latency), except
        // browser controls, which the UI handles.
        let app = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("midi-dispatch".into())
            .spawn(move || {
                while let Ok(ev) = rx.recv() {
                    let Some(app) = app.upgrade() else { break };
                    match ev {
                        MidiEvent::Control(c) => app.control(c),
                        MidiEvent::Raw { bytes, t, port } => app.learn_feed(&port, &bytes, t),
                        MidiEvent::Connected { mapping, .. } => {
                            app.show_drums_for(mapping.as_deref());
                            app.notify(UiEvent::MidiChanged)
                        }
                        MidiEvent::Disconnected { .. } => app.notify(UiEvent::MidiChanged),
                        MidiEvent::NextDeckLayout { port } => app.next_deck_layout(&port),
                    }
                }
            })
            .expect("spawn midi dispatch");
        // Hotplug: a controller unplugged and plugged in again carries on
        // within a second.
        let app = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("midi-hotplug".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let Some(app) = app.upgrade() else { break };
                    if app.shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    let store = app.mappings.lock().expect("mappings lock");
                    if let Some(m) = app.midi.lock().expect("midi lock").as_mut()
                        && !m.refresh_with(|port, connected| app.pick_mapping(&store, port, connected)).is_empty()
                    {
                        app.notify(UiEvent::MidiChanged);
                    }
                }
            })
            .expect("spawn midi hotplug");
    }

    /// The mapping for a newly seen MIDI port: the one last chosen for this
    /// device in the settings, else the first whose device pattern matches,
    /// in the first of its deck layouts no other `connected` controller
    /// uses, so a second X1 drives decks C and D. A remix controller (such
    /// as the F1) without a chosen mapping drives a remix deck: the first
    /// such deck layout whose deck is one.
    fn pick_mapping(
        &self,
        store: &MappingStore,
        port: &str,
        connected: &[(String, Option<String>)],
    ) -> Option<Mapping> {
        match self.settings().midi_mappings.get(port_base_name(port)) {
            Some(name) if name.is_empty() => None,
            Some(name) => store.by_name(name).or_else(|| store.find(port)).cloned(),
            None => {
                let remix_deck = |m: &Mapping| {
                    m.inputs.iter().filter_map(|b| b.target.control()).find(|t| t.control.is_remix()).map(|t| t.unit)
                };
                let first = store.find(port)?;
                let fits = |m: &&Mapping| remix_deck(m).is_none_or(|d| self.is_remix_deck(d));
                let free = |m: &&Mapping| !connected.iter().any(|(p, n)| p != port && n.as_ref() == Some(&m.name));
                let layouts = store.deck_layouts(&first.name);
                let m = layouts
                    .iter()
                    .copied()
                    .find(|m| fits(m) && free(m))
                    .or_else(|| layouts.iter().copied().find(fits))
                    .or_else(|| store.find_all(port).find(fits))
                    .unwrap_or(first);
                Some(m.clone())
            }
        }
    }

    /// The controller on `port` moves on to its mapping's next deck layout,
    /// remembered as if chosen in the settings.
    fn next_deck_layout(&self, port: &str) {
        let current = self.midi_devices().into_iter().find(|(p, _)| p == port).and_then(|(_, m)| m);
        let next = current.and_then(|name| {
            let store = self.mappings.lock().expect("mappings lock");
            store.next_deck_layout(&name).map(|m| (m.name.clone(), m.deck_layout.clone().unwrap_or_default()))
        });
        let Some((name, layout)) = next else { return };
        self.set_port_mapping(port, Some(&name));
        let decks: Vec<String> = layout.chars().map(String::from).collect();
        self.notify(UiEvent::Status(format!("{}: decks {}", port_base_name(port), decks.join(", "))));
    }

    /// After the remix decks changed: controllers on their automatic mapping
    /// move to the deck layout that drives a remix deck.
    pub(crate) fn refollow_remix_decks(&self) {
        let store = self.mappings.lock().expect("mappings lock");
        let chosen = self.settings().midi_mappings;
        let mut midi = self.midi.lock().expect("midi lock");
        let Some(m) = midi.as_mut() else { return };
        let connected = m.connected();
        for (port, current) in &connected {
            if chosen.contains_key(port_base_name(port)) {
                continue;
            }
            let pick = self.pick_mapping(&store, port, &connected);
            if pick.as_ref().map(|p| &p.name) != current.as_ref() {
                let _ = m.set_mapping(port, pick);
            }
        }
    }

    /// Connected MIDI inputs and their mapping names.
    pub fn midi_devices(&self) -> Vec<(String, Option<String>)> {
        self.midi.lock().expect("midi lock").as_ref().map(|m| m.connected()).unwrap_or_default()
    }

    pub fn midi_ports(&self) -> Vec<String> {
        self.midi.lock().expect("midi lock").as_ref().map(|m| m.input_ports()).unwrap_or_default()
    }

    pub fn mapping_names(&self) -> Vec<String> {
        self.mappings.lock().expect("mappings lock").entries().iter().map(|e| e.mapping.name.clone()).collect()
    }

    /// Mapping `name` in each deck layout of its file, as (mapping name,
    /// layout such as "CD"); empty for a mapping with fewer than two.
    pub fn mapping_deck_layouts(&self, name: &str) -> Vec<(String, String)> {
        let store = self.mappings.lock().expect("mappings lock");
        let layouts = store.deck_layouts(name);
        if layouts.len() < 2 {
            return Vec::new();
        }
        layouts.iter().map(|m| (m.name.clone(), m.deck_layout.clone().unwrap_or_default())).collect()
    }

    /// The MIDI channels of mapping `name` and of the mappings it includes,
    /// as (mapping name as written in its file, channel).
    pub fn mapping_channels(&self, name: &str) -> Vec<(String, u8)> {
        self.mappings.lock().expect("mappings lock").channels(name).to_vec()
    }

    /// The controller of mapping `name` (as written in its file) sends on
    /// MIDI `channel`: remembers it, reloads the mappings and gives every
    /// connected controller its mapping again.
    pub fn set_mapping_channel(&self, name: &str, channel: u8) {
        let settings = {
            let mut s = self.settings.write().expect("settings lock");
            s.midi_channels.insert(name.to_owned(), channel.clamp(1, 16));
            s.clone()
        };
        let _ = settings.save(&self.paths.settings_file());
        let mut store = self.mappings.lock().expect("mappings lock");
        *store = MappingStore::load_with_channels(&self.mapping_dirs, &settings.midi_channels);
        if let Some(m) = self.midi.lock().expect("midi lock").as_mut() {
            m.set_hid_layouts(store.hid_layouts());
            for (port, current) in m.connected() {
                if let Some(fresh) = current.and_then(|name| store.by_name(&name).cloned()) {
                    let _ = m.set_mapping(&port, Some(fresh));
                }
            }
        }
        drop(store);
        self.notify(UiEvent::MidiChanged);
    }

    /// Uses mapping `name` (or none) for MIDI input `port`, and remembers
    /// the choice for when the device is plugged in again.
    pub fn set_port_mapping(&self, port: &str, name: Option<&str>) {
        let mapping = name.and_then(|n| self.mappings.lock().expect("mappings lock").by_name(n).cloned());
        let settings = {
            let mut s = self.settings.write().expect("settings lock");
            let chosen = mapping.as_ref().map(|m| m.name.clone()).unwrap_or_default();
            s.midi_mappings.insert(port_base_name(port).to_owned(), chosen);
            s.clone()
        };
        let _ = settings.save(&self.paths.settings_file());
        if let Some(m) = self.midi.lock().expect("midi lock").as_mut() {
            let result = if m.connected().iter().any(|(p, _)| p == port) {
                m.set_mapping(port, mapping)
            } else {
                m.connect(port, mapping)
            };
            if let Err(e) = result {
                self.notify(UiEvent::Status(format!("MIDI: {e:?}")));
            }
        }
        self.notify(UiEvent::MidiChanged);
    }

    /// MIDI learn: while on, clicking a control selects it, moving a
    /// hardware control binds it.
    pub fn set_learn(&self, on: bool) {
        let mut learn = self.learn.lock().expect("learn lock");
        *learn = on.then(|| LearnSession::new(rille_core::ControlTarget::global(Control::Crossfader)));
        if let Some(m) = self.midi.lock().expect("midi lock").as_ref() {
            m.set_raw_events(on);
        }
        self.notify(UiEvent::MidiLearned(if on {
            "Click a control, then move it on your controller".into()
        } else {
            String::new()
        }));
    }

    pub fn learning(&self) -> bool {
        self.learn.lock().expect("learn lock").is_some()
    }

    fn learn_feed(&self, port: &str, bytes: &[u8], t: Instant) {
        let binding = {
            let mut learn = self.learn.lock().expect("learn lock");
            let Some(session) = learn.as_mut() else { return };
            session.feed(bytes, t);
            session.result()
        };
        let Some(binding) = binding else { return };
        let port_name = port.to_string();
        let mut mapping = self
            .midi
            .lock()
            .expect("midi lock")
            .as_ref()
            .and_then(|m| m.connected().into_iter().find(|(p, _)| *p == port_name).and_then(|(_, name)| name))
            .and_then(|name| self.mappings.lock().expect("mappings lock").by_name(&name).cloned())
            .unwrap_or_else(|| rille_midi::Mapping::new(format!("{port_name} (learned)"), regex_escape(&port_name)));
        let text = format!("Mapped {}", binding.target);
        mapping.set_input(binding);
        if let Ok(path) = rille_midi::store::save_to_dir(&mapping, &self.paths.user_mappings()) {
            self.mappings.lock().expect("mappings lock").push(mapping.clone(), path);
        }
        if let Some(m) = self.midi.lock().expect("midi lock").as_mut() {
            let _ = m.set_mapping(&port_name, Some(mapping));
        }
        if let Some(s) = self.learn.lock().expect("learn lock").as_mut() {
            s.clear();
        }
        self.notify(UiEvent::MidiLearned(text));
    }
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    out.push('^');
    for c in s.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Times the file reading of an import or scan; the rate is measured from
/// the first file read, after the folder walk.
struct ReadClock {
    started: Instant,
    reading: Option<Instant>,
}

impl ReadClock {
    fn new() -> Self {
        Self { started: Instant::now(), reading: None }
    }

    fn timing(&mut self, done: usize, total: usize) -> Timing {
        let now = Instant::now();
        let reading = *self.reading.get_or_insert(now);
        Timing { elapsed: now - self.started, remaining: timing::linear_estimate(now - reading, done, total) }
    }
}

/// Creates (or reuses) the playlists for an imported folder tree under
/// `parent`: a playlist for a folder without subfolders, else a playlist
/// folder with the folder's own files in a playlist of the same name.
/// Returns how many entries were added.
fn mirror_folder(lib: &mut Library, tree: &FolderTree, parent: Option<PlaylistId>) -> rille_library::Result<usize> {
    let ids = |lib: &Library, files: &[PathBuf]| -> Vec<TrackId> {
        files.iter().filter_map(|p| lib.track_by_path(p).ok().flatten()).collect()
    };
    if tree.children.is_empty() {
        let list = lib.playlist_named(parent, &tree.name, false)?;
        let tracks = ids(lib, &tree.files);
        return lib.add_missing_to_playlist(list, &tracks);
    }
    let folder = lib.playlist_named(parent, &tree.name, true)?;
    let mut added = 0;
    if !tree.files.is_empty() {
        let list = lib.playlist_named(Some(folder), &tree.name, false)?;
        let tracks = ids(lib, &tree.files);
        added += lib.add_missing_to_playlist(list, &tracks)?;
    }
    for c in &tree.children {
        added += mirror_folder(lib, c, Some(folder))?;
    }
    Ok(added)
}

fn analysis_config(s: &Settings) -> AnalysisConfig {
    AnalysisConfig { bpm_range: (s.bpm_min, s.bpm_max) }
}

/// Start of the kick attack within 50 ms of `secs` (the broadband attack for
/// tracks without a kick), where the analyzer would put the beat. `None` if
/// nothing clearly starts there.
fn attack_near(audio: &TrackAudio, secs: f64) -> Option<f64> {
    use rille_analysis::refine::{Band, TransientFinder};
    // At least 4 dB, so noise or a fading tail doesn't count as an attack.
    const MIN_STRENGTH: f64 = 0.92;
    const RADIUS: f64 = 0.05;
    let sr = f64::from(audio.sample_rate.max(1));
    let first = ((secs - 0.5) * sr).max(0.0) as usize;
    let last = (((secs + 0.5) * sr).max(0.0) as usize).min(audio.frames.len());
    if last <= first {
        return None;
    }
    let mono: Vec<f32> = audio.frames[first..last].iter().map(|[l, r]| 0.5 * (l + r)).collect();
    let offset = first as f64 / sr;
    [Band::Low, Band::Broad].into_iter().find_map(|band| {
        let t = TransientFinder::new(&mono, sr, band).attack(secs - offset, RADIUS)?;
        (t.strength >= MIN_STRENGTH).then_some(t.secs + offset)
    })
}

fn hotcues_from(cues: &TrackCues) -> [Option<Hotcue>; HOTCUES] {
    std::array::from_fn(|slot| {
        cues.hotcue(slot as u8).map(|c| Hotcue { secs: c.start_secs, kind: c.kind, len_secs: c.len_secs })
    })
}

pub fn sort_rows(rows: &mut [TrackRow], key: SortKey, descending: bool) {
    let lower = |s: &str| s.to_lowercase();
    rows.sort_by(|a, b| {
        let o = match key {
            SortKey::Artist => {
                lower(&a.artist).cmp(&lower(&b.artist)).then_with(|| lower(&a.title).cmp(&lower(&b.title)))
            }
            SortKey::Title => lower(&a.title).cmp(&lower(&b.title)),
            SortKey::Album => lower(&a.album).cmp(&lower(&b.album)),
            SortKey::Label => lower(&a.label).cmp(&lower(&b.label)),
            SortKey::Genre => lower(&a.genre).cmp(&lower(&b.genre)),
            SortKey::Bpm => a.bpm.unwrap_or(0.0).total_cmp(&b.bpm.unwrap_or(0.0)),
            SortKey::Key => {
                let k =
                    |r: &TrackRow| r.key.map_or(99, |k| u16::from(k.camelot_number()) * 2 + u16::from(!k.is_minor()));
                k(a).cmp(&k(b))
            }
            SortKey::Rating => a.rating.cmp(&b.rating),
            SortKey::Duration => a.duration_secs.total_cmp(&b.duration_secs),
            SortKey::Added => a.date_added.cmp(&b.date_added),
            SortKey::PlayCount => a.play_count.cmp(&b.play_count),
            SortKey::FileName => a.path.file_name().cmp(&b.path.file_name()),
            SortKey::Remixer => lower(&a.remixer).cmp(&lower(&b.remixer)),
            SortKey::Comment => lower(&a.comment).cmp(&lower(&b.comment)),
            SortKey::Year => a.year.cmp(&b.year),
            SortKey::Bitrate => a.bitrate.cmp(&b.bitrate),
            SortKey::SampleRate => a.sample_rate.cmp(&b.sample_rate),
            SortKey::FileSize => a.file_size.cmp(&b.file_size),
            SortKey::LastPlayed => a.last_played.cmp(&b.last_played),
            SortKey::Path => a.path.cmp(&b.path),
        };
        if descending { o.reverse() } else { o }
    });
}

/// Scope helper for the UI: is this control handled per deck?
pub fn is_deck_control(c: Control) -> bool {
    c.scope() == Scope::Deck
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Silence, then a pitch-swept kick at `at` seconds.
    fn kick_at(sr: u32, at: f64) -> TrackAudio {
        let sr_f = f64::from(sr);
        let mut frames = vec![[0.0f32; 2]; (3.0 * sr_f) as usize];
        let start = (at * sr_f).round() as usize;
        let mut phase = 0.0f64;
        for (k, f) in frames[start..].iter_mut().enumerate().take((0.4 * sr_f) as usize) {
            let t = k as f64 / sr_f;
            phase += 2.0 * std::f64::consts::PI * (45.0 + 100.0 * (-t * 30.0).exp()) / sr_f;
            let v = (phase.sin() * (-t * 12.0).exp() * 0.8) as f32;
            *f = [v, v];
        }
        TrackAudio { sample_rate: sr, frames }
    }

    #[test]
    fn attack_near_snaps_onto_the_kick() {
        let audio = kick_at(44_100, 1.2);
        for pos in [1.17, 1.2, 1.23] {
            let t = attack_near(&audio, pos).expect("kick found");
            assert!((t - 1.2).abs() < 0.001, "from {pos}: {t}");
        }
        // Nothing starts in silence; the play position is used as it is.
        assert!(attack_near(&audio, 0.5).is_none());
        assert!(attack_near(&audio, 2.9).is_none());
    }

    #[test]
    fn a_second_controller_takes_the_free_deck_layout() {
        let dir = tempfile::tempdir().unwrap();
        let opts = StartOptions {
            paths: Paths::under(dir.path()),
            audio: false,
            midi: false,
            bundled_mappings: Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mappings")),
        };
        let app = App::start(opts).unwrap();
        let (first, second) = ("Traktor Kontrol X1 MK2 HID (AAAA)", "Traktor Kontrol X1 MK2 HID (BBBB)");
        let (ab, cd) = ("Traktor Kontrol X1 MK2 (AB)".to_owned(), "Traktor Kontrol X1 MK2 (CD)".to_owned());
        let pick = |port: &str, connected: &[(String, Option<String>)]| {
            app.pick_mapping(&app.mappings.lock().unwrap(), port, connected).map(|m| m.name)
        };
        assert_eq!(pick(first, &[]), Some(ab.clone()));
        assert_eq!(pick(second, &[(first.into(), Some(ab.clone()))]), Some(cd.clone()));
        // Its own mapping does not count as taken; with both taken, the first.
        assert_eq!(pick(first, &[(first.into(), Some(ab.clone()))]), Some(ab.clone()));
        let both = [(first.into(), Some(ab.clone())), (second.into(), Some(cd.clone()))];
        assert_eq!(pick("Traktor Kontrol X1 MK2 HID (CCCC)", &both), Some(ab.clone()));
        // A remembered choice wins.
        let mut s = app.settings();
        s.midi_mappings.insert(second.into(), ab.clone());
        app.set_settings(s);
        assert_eq!(pick(second, &[(first.into(), Some(ab.clone()))]), Some(ab.clone()));
        assert_eq!(app.mapping_deck_layouts(&ab), [(ab.clone(), "AB".to_owned()), (cd, "CD".to_owned())]);
        app.shutdown();
    }
}
