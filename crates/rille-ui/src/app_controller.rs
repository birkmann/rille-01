//! Global state and actions for QML: header, clock, mixer master section,
//! FX units, settings, library actions.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(f64, cpu_load, cxx_name = "cpuLoad")]
        #[qproperty(f64, master_peak_l, cxx_name = "masterPeakL")]
        #[qproperty(f64, master_peak_r, cxx_name = "masterPeakR")]
        #[qproperty(f64, limiter_reduction, cxx_name = "limiterReduction")]
        #[qproperty(f64, clock_bpm, cxx_name = "clockBpm")]
        #[qproperty(f64, clock_beat, cxx_name = "clockBeat")]
        #[qproperty(i32, master_deck, cxx_name = "masterDeck")]
        #[qproperty(bool, quantize)]
        #[qproperty(bool, snap)]
        #[qproperty(bool, limiter)]
        #[qproperty(f64, crossfader)]
        #[qproperty(f64, main_level, cxx_name = "mainLevel")]
        #[qproperty(f64, cue_mix, cxx_name = "cueMix")]
        #[qproperty(f64, cue_volume, cxx_name = "cueVolume")]
        #[qproperty(QString, status)]
        #[qproperty(QString, analysis_text, cxx_name = "analysisText")]
        #[qproperty(QString, audio_text, cxx_name = "audioText")]
        #[qproperty(bool, learning)]
        #[qproperty(QString, learn_text, cxx_name = "learnText")]
        #[qproperty(i32, library_revision, cxx_name = "libraryRevision")]
        #[qproperty(i32, midi_revision, cxx_name = "midiRevision")]
        #[qproperty(QString, fx_json, cxx_name = "fxJson")]
        #[qproperty(QString, browser_action, cxx_name = "browserAction")]
        #[qproperty(i32, browser_action_seq, cxx_name = "browserActionSeq")]
        /// Bumped when single tracks changed (see `TrackListModel::updateChanged`).
        #[qproperty(i32, tracks_revision, cxx_name = "tracksRevision")]
        #[qproperty(i32, analysis_done, cxx_name = "analysisDone")]
        #[qproperty(i32, analysis_total, cxx_name = "analysisTotal")]
        #[qproperty(i32, analysis_failed, cxx_name = "analysisFailed")]
        #[qproperty(bool, analysis_paused, cxx_name = "analysisPaused")]
        #[qproperty(QString, analysis_current, cxx_name = "analysisCurrent")]
        /// "3:21 elapsed · ~14:05 left".
        #[qproperty(QString, analysis_time, cxx_name = "analysisTime")]
        /// "Reading files 120/800" while importing or scanning.
        #[qproperty(QString, import_text, cxx_name = "importText")]
        /// Tempo fader range (0.08 = ±8 %).
        #[qproperty(f64, tempo_range, cxx_name = "tempoRange")]
        /// Decks on screen: 2 or 4.
        #[qproperty(i32, deck_count, cxx_name = "deckCount")]
        /// Main output meter in the title bar.
        #[qproperty(bool, header_meter, cxx_name = "headerMeter")]
        /// Settings → Audio: the on-screen mixer and crossfader are hidden.
        #[qproperty(bool, mixer_hidden, cxx_name = "mixerHidden")]
        /// External mixing: the deck on each hardware mixer channel, e.g.
        /// "CABD"; empty with the internal mixer.
        #[qproperty(QString, mixer_channels, cxx_name = "mixerChannels")]
        /// A MIDI controller is connected.
        #[qproperty(bool, midi_connected, cxx_name = "midiConnected")]
        /// A laptop running from its battery (charger unplugged).
        #[qproperty(bool, on_battery, cxx_name = "onBattery")]
        /// Battery charge 0..100, -1 without a system battery.
        #[qproperty(i32, battery_percent, cxx_name = "batteryPercent")]
        /// Estimated runtime left on battery, -1 when unknown.
        #[qproperty(i32, battery_minutes, cxx_name = "batteryMinutes")]
        /// Settings → Library: the browser shows track suggestions.
        #[qproperty(bool, suggestions_enabled, cxx_name = "suggestionsEnabled")]
        /// Bumped when the suggestions are for another track.
        #[qproperty(i32, suggestions_revision, cxx_name = "suggestionsRevision")]
        /// Bumped when Beatport sign-in, a Beatport list or the playlists
        /// changed.
        #[qproperty(i32, beatport_revision, cxx_name = "beatportRevision")]
        /// Bumped when the download queue moved on (row indicators).
        #[qproperty(i32, beatport_download_revision, cxx_name = "beatportDownloadRevision")]
        /// The signed-in Beatport user, empty when signed out.
        #[qproperty(QString, beatport_account, cxx_name = "beatportAccount")]
        /// Beatport tracks are being downloaded for offline use.
        #[qproperty(bool, beatport_downloading, cxx_name = "beatportDownloading")]
        /// "Downloading 3 / 29 · 5.6 MB/s".
        #[qproperty(QString, beatport_download_text, cxx_name = "beatportDownloadText")]
        /// Share of the batch done, 0..1.
        #[qproperty(f64, beatport_download_fraction, cxx_name = "beatportDownloadFraction")]
        type AppController = super::AppControllerRust;

        /// Downloads a Beatport playlist of the user for offline use.
        #[qinvokable]
        #[cxx_name = "downloadBeatportPlaylist"]
        fn download_beatport_playlist(self: &AppController, playlist: i64);

        #[qinvokable]
        #[cxx_name = "cancelBeatportDownloads"]
        fn cancel_beatport_downloads(self: &AppController);

        /// Signs in to Beatport in the background (the password is not kept).
        #[qinvokable]
        #[cxx_name = "beatportLogin"]
        fn beatport_login(self: &AppController, username: &QString, password: &QString);

        #[qinvokable]
        #[cxx_name = "beatportLogout"]
        fn beatport_logout(self: &AppController);

        /// Loads a Beatport catalog track (by Beatport id) onto a deck.
        #[qinvokable]
        #[cxx_name = "loadBeatport"]
        fn load_beatport(self: &AppController, deck: i32, beatport_id: i64);

        /// Fetches the user's Beatport playlists again.
        #[qinvokable]
        #[cxx_name = "refreshBeatportPlaylists"]
        fn refresh_beatport_playlists(self: &AppController);

        /// Deletes the cached files of streamed tracks (not those on a deck);
        /// `with_offline` also those kept offline.
        #[qinvokable]
        #[cxx_name = "clearBeatportCache"]
        fn clear_beatport_cache(self: &AppController, with_offline: bool);

        /// "12 tracks · 480 MB, 8 kept offline (310 MB)" for the settings.
        #[qinvokable]
        #[cxx_name = "beatportCacheText"]
        fn beatport_cache_text(self: &AppController) -> QString;

        /// Opens a web page in the desktop's browser.
        #[qinvokable]
        #[cxx_name = "openWebPage"]
        fn open_web_page(self: &AppController, url: &QString);

        #[qinvokable]
        #[cxx_name = "pauseAnalysis"]
        fn pause_analysis(self: &AppController, paused: bool);

        #[qinvokable]
        #[cxx_name = "cancelAnalysis"]
        fn cancel_analysis(self: &AppController);

        /// Adds the audio files of a folder (path token) to the collection.
        #[qinvokable]
        #[cxx_name = "importFolder"]
        fn import_folder(self: &AppController, token: i64, recursive: bool, analyze: bool);

        /// Imports a folder (path token) and mirrors it as a playlist of the
        /// same name; with `recursive`, subfolders as a playlist folder.
        #[qinvokable]
        #[cxx_name = "importFolderAsPlaylist"]
        fn import_folder_as_playlist(self: &AppController, token: i64, recursive: bool, analyze: bool);

        /// Analyzes a folder's files, importing the new ones first.
        #[qinvokable]
        #[cxx_name = "analyzeFolder"]
        fn analyze_folder(self: &AppController, token: i64, recursive: bool, force: bool);

        #[qinvokable]
        #[cxx_name = "addMusicFolderToken"]
        fn add_music_folder_token(self: &AppController, token: i64);

        /// Shows a folder in the desktop's file manager.
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: &AppController, token: i64);

        /// Loads a file by path token (drag and drop from the explorer).
        #[qinvokable]
        #[cxx_name = "loadPath"]
        fn load_path(self: &AppController, deck: i32, token: i64);

        /// Poll engine and app state; call once per frame.
        #[qinvokable]
        fn tick(self: Pin<&mut AppController>);

        /// Button press/release on a control target such as "deck.A.play".
        #[qinvokable]
        fn press(self: &AppController, target: &QString, down: bool);

        /// Absolute value 0..1 for knobs and faders.
        #[qinvokable]
        #[cxx_name = "setValue"]
        fn set_value(self: &AppController, target: &QString, value: f64);

        /// Relative change (jog wheels, encoders).
        #[qinvokable]
        fn nudge(self: &AppController, target: &QString, delta: f64);

        #[qinvokable]
        #[cxx_name = "loadTrack"]
        fn load_track(self: &AppController, deck: i32, track_id: i64);

        #[qinvokable]
        #[cxx_name = "loadUrl"]
        fn load_url(self: &AppController, deck: i32, url: &QUrl);

        #[qinvokable]
        fn eject(self: &AppController, deck: i32);

        /// "move" (arg ms), "double", "halve", "beat", "downbeat", "barstart",
        /// "tap", "lock" (arg 0/1), "reset".
        #[qinvokable]
        #[cxx_name = "gridEdit"]
        fn grid_edit(self: &AppController, deck: i32, op: &QString, arg: f64);

        #[qinvokable]
        #[cxx_name = "setLearn"]
        fn set_learn(self: &AppController, on: bool);

        #[qinvokable]
        #[cxx_name = "addMusicFolder"]
        fn add_music_folder(self: &AppController, url: &QUrl);

        #[qinvokable]
        #[cxx_name = "removeMusicFolder"]
        fn remove_music_folder(self: &AppController, path: &QString);

        #[qinvokable]
        fn rescan(self: &AppController);

        #[qinvokable]
        #[cxx_name = "importNml"]
        fn import_nml(self: Pin<&mut AppController>, url: &QUrl);

        /// Settings as JSON for the settings dialog.
        #[qinvokable]
        #[cxx_name = "settingsJson"]
        fn settings_json(self: &AppController) -> QString;

        /// Changes one setting (name as in settings.toml, value as text).
        #[qinvokable]
        #[cxx_name = "setSetting"]
        fn set_setting(self: &AppController, name: &QString, value: &QString);

        #[qinvokable]
        #[cxx_name = "audioDevicesJson"]
        fn audio_devices_json(self: &AppController) -> QString;

        /// MIDI inputs, connected devices and mapping names as JSON.
        #[qinvokable]
        #[cxx_name = "midiJson"]
        fn midi_json(self: &AppController) -> QString;

        #[qinvokable]
        #[cxx_name = "setPortMapping"]
        fn set_port_mapping(self: &AppController, port: &QString, mapping: &QString);

        #[qinvokable]
        #[cxx_name = "createPlaylist"]
        fn create_playlist(self: &AppController, name: &QString) -> i64;

        #[qinvokable]
        #[cxx_name = "addToPlaylist"]
        fn add_to_playlist(self: &AppController, playlist: i64, track_id: i64);

        #[qinvokable]
        #[cxx_name = "deletePlaylist"]
        fn delete_playlist(self: &AppController, playlist: i64);

        #[qinvokable]
        #[cxx_name = "setRating"]
        fn set_rating(self: &AppController, track_id: i64, stars: i32);

        #[qinvokable]
        #[cxx_name = "setClockTempo"]
        fn set_clock_tempo(self: &AppController, bpm: f64);

        #[qinvokable]
        #[cxx_name = "setInternalMaster"]
        fn set_internal_master(self: &AppController);

        /// Effect names for the FX selectors, as JSON.
        #[qinvokable]
        #[cxx_name = "effectNamesJson"]
        fn effect_names_json(self: &AppController) -> QString;

        #[qinvokable]
        fn version(self: &AppController) -> QString;

        /// Source code URL for the About dialog (empty when unknown).
        #[qinvokable]
        fn repository(self: &AppController) -> QString;

        #[qinvokable]
        #[cxx_name = "smokeTest"]
        fn smoke_test(self: &AppController) -> bool;

        #[qinvokable]
        #[cxx_name = "screenshotPath"]
        fn screenshot_path(self: &AppController) -> QString;

        /// Milliseconds before the screenshot: `--delay=<secs>`, default 4 s.
        #[qinvokable]
        #[cxx_name = "screenshotDelay"]
        fn screenshot_delay(self: &AppController) -> i32;

        /// Whether the command line has `--<name>` (e.g. "settings").
        #[qinvokable]
        #[cxx_name = "hasArg"]
        fn has_arg(self: &AppController, name: &QString) -> bool;

        /// Tracks in the collection.
        #[qinvokable]
        #[cxx_name = "trackCount"]
        fn track_count(self: &AppController) -> i32;

        /// `--size=WxH` as "WxH", or empty.
        #[qinvokable]
        #[cxx_name = "windowSize"]
        fn window_size(self: &AppController) -> QString;

        /// A path token's path, for display.
        #[qinvokable]
        #[cxx_name = "tokenPathText"]
        fn token_path_text(self: &AppController, token: i64) -> QString;

        /// Path token of `--browse=<folder>` (open in the explorer), or −1.
        #[qinvokable]
        #[cxx_name = "browseToken"]
        fn browse_token(self: &AppController) -> i64;
    }
}

use core::pin::Pin;
use std::path::PathBuf;
use std::time::Instant;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};
use rille_app::{GridEdit, KeyNotation, MixingMode, Timing, UiEvent, WaveformStyle};
use rille_core::{ControlEvent, ControlTarget, ControlValue};
use rille_engine::Command;

use crate::global::{app, token_path};

#[derive(Default)]
pub struct AppControllerRust {
    cpu_load: f64,
    master_peak_l: f64,
    master_peak_r: f64,
    limiter_reduction: f64,
    clock_bpm: f64,
    clock_beat: f64,
    master_deck: i32,
    quantize: bool,
    snap: bool,
    limiter: bool,
    crossfader: f64,
    main_level: f64,
    cue_mix: f64,
    cue_volume: f64,
    status: QString,
    analysis_text: QString,
    audio_text: QString,
    learning: bool,
    learn_text: QString,
    library_revision: i32,
    midi_revision: i32,
    fx_json: QString,
    browser_action: QString,
    browser_action_seq: i32,
    tracks_revision: i32,
    analysis_done: i32,
    analysis_total: i32,
    analysis_failed: i32,
    analysis_paused: bool,
    analysis_current: QString,
    analysis_time: QString,
    import_text: QString,
    /// The last import progress and when it came, to keep the clock
    /// running between reports.
    import: Option<ImportReport>,
    tempo_range: f64,
    deck_count: i32,
    header_meter: bool,
    mixer_hidden: bool,
    mixer_channels: QString,
    midi_connected: bool,
    on_battery: bool,
    battery_percent: i32,
    battery_minutes: i32,
    suggestions_enabled: bool,
    suggestions_revision: i32,
    beatport_revision: i32,
    beatport_download_revision: i32,
    beatport_account: QString,
    beatport_downloading: bool,
    beatport_download_text: QString,
    beatport_download_fraction: f64,
}

struct ImportReport {
    done: usize,
    total: usize,
    timing: Timing,
    at: Instant,
}

impl ImportReport {
    /// "Reading files 120/800 · 0:42 elapsed · ~3:10 left", as of now.
    fn text(&self) -> String {
        let since = self.at.elapsed();
        let now = Timing {
            elapsed: self.timing.elapsed + since,
            remaining: self.timing.remaining.map(|r| r.saturating_sub(since)),
        };
        format!("Reading files {}/{} · {}", self.done, self.total, now.describe(false))
    }
}

fn target(s: &QString) -> Option<ControlTarget> {
    s.to_string().parse().ok()
}

fn send(t: &QString, value: ControlValue) {
    if let (Some(app), Some(target)) = (app(), target(t)) {
        app.control(ControlEvent { target, value });
    }
}

pub fn url_path(url: &QUrl) -> PathBuf {
    let s = url.to_local_file().map(|q| q.to_string()).unwrap_or_default();
    PathBuf::from(if s.is_empty() { url.to_string() } else { s })
}

impl qobject::AppController {
    fn pause_analysis(&self, paused: bool) {
        if let Some(app) = app() {
            app.set_analysis_paused(paused);
        }
    }

    fn cancel_analysis(&self) {
        if let Some(app) = app() {
            app.cancel_analysis();
        }
    }

    fn import_folder(&self, token: i64, recursive: bool, analyze: bool) {
        if let (Some(app), Some(dir)) = (app(), token_path(token)) {
            app.import_folder(&dir, recursive, analyze);
        }
    }

    fn import_folder_as_playlist(&self, token: i64, recursive: bool, analyze: bool) {
        if let (Some(app), Some(dir)) = (app(), token_path(token)) {
            app.import_folder_as_playlist(&dir, recursive, analyze);
        }
    }

    fn analyze_folder(&self, token: i64, recursive: bool, force: bool) {
        if let (Some(app), Some(dir)) = (app(), token_path(token)) {
            app.analyze_paths(rille_app::explorer::audio_files(&dir, recursive), force);
        }
    }

    fn add_music_folder_token(&self, token: i64) {
        if let (Some(app), Some(dir)) = (app(), token_path(token)) {
            app.add_music_folder(dir);
        }
    }

    fn open_folder(&self, token: i64) {
        if let Some(dir) = token_path(token) {
            // The desktop decides which file manager opens; errors only
            // mean there is none.
            let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
        }
    }

    fn load_path(&self, deck: i32, token: i64) {
        if let (Some(app), Some(p)) = (app(), token_path(token)) {
            app.load_file(deck.clamp(0, 3) as u8, &p);
        }
    }

    fn beatport_login(&self, username: &QString, password: &QString) {
        if let Some(app) = app() {
            app.beatport_login(username.to_string(), password.to_string());
        }
    }

    fn beatport_logout(&self) {
        if let Some(app) = app() {
            app.beatport_logout();
        }
    }

    fn load_beatport(&self, deck: i32, beatport_id: i64) {
        if let Some(app) = app() {
            app.load_beatport(deck.clamp(0, 3) as u8, beatport_id);
        }
    }

    fn refresh_beatport_playlists(&self) {
        if let Some(app) = app() {
            app.beatport_refresh_playlists();
        }
    }

    fn clear_beatport_cache(&self, with_offline: bool) {
        if let Some(app) = app() {
            app.clear_beatport_cache(with_offline);
        }
    }

    fn beatport_cache_text(&self) -> QString {
        let u = app().map(|a| a.beatport_cache_usage()).unwrap_or_default();
        let size = |b: u64| {
            if b >= 1 << 30 { format!("{:.1} GB", b as f64 / f64::from(1 << 30)) } else { format!("{} MB", b >> 20) }
        };
        QString::from(format!(
            "{} tracks · {}, of those {} kept offline ({})",
            u.files,
            size(u.bytes),
            u.offline_files,
            size(u.offline_bytes)
        ))
    }

    fn download_beatport_playlist(&self, playlist: i64) {
        if let Some(app) = app() {
            app.beatport_download_list(rille_app::BeatportList::Playlist(playlist));
        }
    }

    fn cancel_beatport_downloads(&self) {
        if let Some(app) = app() {
            app.beatport_cancel_downloads();
        }
    }

    fn open_web_page(&self, url: &QString) {
        let url = url.to_string();
        if url.starts_with("https://") {
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        }
    }

    fn tick(mut self: Pin<&mut Self>) {
        let Some(app) = app() else { return };
        app.tick();
        let s = app.snapshot();
        self.as_mut().set_cpu_load(f64::from(s.cpu_load));
        self.as_mut().set_master_peak_l(f64::from(s.master_meter[0]));
        self.as_mut().set_master_peak_r(f64::from(s.master_meter[1]));
        self.as_mut().set_limiter_reduction(f64::from(s.limiter_reduction_db));
        self.as_mut().set_clock_bpm((s.clock_bpm * 100.0).round() / 100.0);
        self.as_mut().set_clock_beat(s.clock_beat);
        self.as_mut().set_master_deck(s.master_deck.map_or(-1, i32::from));
        self.as_mut().set_quantize(s.quantize);
        self.as_mut().set_snap(s.snap);
        self.as_mut().set_limiter(s.limiter);
        self.as_mut().set_crossfader(f64::from(s.crossfader));
        self.as_mut().set_main_level(f64::from(s.main_level));
        self.as_mut().set_cue_mix(f64::from(s.cue_mix));
        self.as_mut().set_cue_volume(f64::from(s.cue_volume));
        let names = fx_names();
        let fx: Vec<String> =
            s.fx.iter()
                .map(|u| {
                    format!(
                        r#"{{"on":{},"dryWet":{:.4},"effects":[{}],"knobs":[{}],"buttons":[{}],"names":[{}]}}"#,
                        u.on,
                        u.dry_wet,
                        u.effects.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(","),
                        // Group mode: the slot knobs are amounts, the ON buttons switch slots.
                        u.amount.iter().map(|k| format!("{k:.4}")).collect::<Vec<_>>().join(","),
                        u.enabled.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(","),
                        u.effects
                            .iter()
                            .map(|e| format!("\"{}\"", names.get(*e).copied().unwrap_or("?")))
                            .collect::<Vec<_>>()
                            .join(","),
                    )
                })
                .collect();
        self.as_mut().set_fx_json(QString::from(format!("[{}]", fx.join(","))));
        let a = app.audio_status();
        let external = app.external_mixing();
        let audio = match &a.error {
            Some(_) => "No audio output".to_string(),
            None => format!(
                "{} · {} Hz · {} frames · {} channels{}",
                a.device,
                a.sample_rate,
                a.buffer_frames.map_or("auto".into(), |b| b.to_string()),
                a.channels,
                if external { " · external mixer" } else { "" }
            ),
        };
        self.as_mut().set_audio_text(QString::from(audio));
        self.as_mut().set_learning(app.learning());
        let settings = app.settings();
        self.as_mut().set_tempo_range(settings.tempo_range);
        self.as_mut().set_deck_count(i32::from(settings.deck_count));
        self.as_mut().set_header_meter(settings.header_meter);
        self.as_mut().set_mixer_hidden(!settings.show_mixer);
        self.as_mut().set_suggestions_enabled(settings.suggestions);
        let channels = if external { settings.mixer_channels.to_uppercase() } else { String::new() };
        if *self.mixer_channels() != QString::from(&channels) {
            self.as_mut().set_mixer_channels(QString::from(channels));
        }
        self.as_mut().set_midi_connected(!app.midi_devices().is_empty());
        let battery = crate::power::current();
        self.as_mut().set_on_battery(battery.is_some_and(|b| b.on_battery));
        self.as_mut().set_battery_percent(battery.map_or(-1, |b| b.percent.round() as i32));
        self.as_mut().set_battery_minutes(battery.and_then(|b| b.minutes_left).map_or(-1, |m| m as i32));
        let account = QString::from(app.beatport_account().unwrap_or_default());
        if *self.beatport_account() != account {
            self.as_mut().set_beatport_account(account);
        }
        let dl = app.beatport_downloads();
        self.as_mut().set_beatport_downloading(dl.running);
        if dl.running {
            let finished = dl.done + dl.failed;
            let text = format!(
                "Downloading {} / {} · {:.1} MB/s",
                (finished + 1).min(dl.total),
                dl.total,
                dl.bytes_per_sec / 1_048_576.0
            );
            if *self.beatport_download_text() != QString::from(&text) {
                self.as_mut().set_beatport_download_text(QString::from(text));
            }
            let fraction = (finished as f64 + f64::from(dl.current)) / dl.total.max(1) as f64;
            self.as_mut().set_beatport_download_fraction(fraction.min(1.0));
        }

        for ev in app.poll_ui_events() {
            match ev {
                UiEvent::Status(t) => self.as_mut().set_status(QString::from(t)),
                UiEvent::BeatportChanged => {
                    let r = *self.beatport_revision() + 1;
                    self.as_mut().set_beatport_revision(r);
                }
                UiEvent::BeatportDownloads => {
                    let r = *self.beatport_download_revision() + 1;
                    self.as_mut().set_beatport_download_revision(r);
                }
                UiEvent::LibraryChanged => {
                    let r = *self.library_revision() + 1;
                    self.as_mut().set_library_revision(r);
                }
                UiEvent::AnalysisProgress(p) => {
                    let t =
                        if p.idle() { String::new() } else { format!("Analyzing {}/{}", p.done + p.failed, p.total) };
                    self.as_mut().set_analysis_text(QString::from(t));
                    let idle = p.idle();
                    self.as_mut().set_analysis_done(p.done as i32);
                    self.as_mut().set_analysis_total(if idle { 0 } else { p.total as i32 });
                    self.as_mut().set_analysis_failed(p.failed as i32);
                    self.as_mut().set_analysis_paused(p.paused);
                    self.as_mut().set_analysis_current(QString::from(p.current));
                }
                UiEvent::TracksChanged(ids) => {
                    crate::global::push_changed(&ids);
                    let r = *self.tracks_revision() + 1;
                    self.as_mut().set_tracks_revision(r);
                }
                UiEvent::ImportProgress { done, total, timing } => {
                    self.as_mut().rust_mut().import =
                        (total > 0).then(|| ImportReport { done, total, timing, at: Instant::now() });
                }
                UiEvent::MidiChanged => {
                    let r = *self.midi_revision() + 1;
                    self.as_mut().set_midi_revision(r);
                }
                UiEvent::MidiLearned(t) => self.as_mut().set_learn_text(QString::from(t)),
                UiEvent::Browser(ev) => {
                    let action = match (ev.target.control, ev.value) {
                        (rille_core::Control::LoadSelected, ControlValue::Press(true)) => {
                            format!("load:{}", ev.target.unit)
                        }
                        (rille_core::Control::BrowserScroll, ControlValue::Delta(d)) => format!("scroll:{d}"),
                        (rille_core::Control::BrowserTreeScroll, ControlValue::Delta(d)) => format!("tree:{d}"),
                        (rille_core::Control::BrowserToggleNode, ControlValue::Press(true)) => "toggle".into(),
                        // BROWSE + pad on a remix controller: the selected
                        // track into that pad's cell.
                        (rille_core::Control::RemixPadLoad(pad), ControlValue::Press(true)) => {
                            let deck = ev.target.unit;
                            let Some(cell) = app.remix_pad_cell(deck, pad) else { continue };
                            format!("cell:{deck}:{cell}")
                        }
                        _ => continue,
                    };
                    self.as_mut().set_browser_action(QString::from(action));
                    let seq = *self.browser_action_seq() + 1;
                    self.as_mut().set_browser_action_seq(seq);
                }
                UiEvent::SuggestionsChanged => {
                    let r = *self.suggestions_revision() + 1;
                    self.as_mut().set_suggestions_revision(r);
                }
                UiEvent::DeckChanged(_) | UiEvent::AudioChanged => {}
            }
        }

        // The clocks run between progress reports.
        let t = if *self.analysis_total() > 0 {
            app.analysis_timing().describe(*self.analysis_paused())
        } else {
            String::new()
        };
        self.as_mut().set_analysis_time(QString::from(t));
        let t = self.import.as_ref().map_or_else(String::new, ImportReport::text);
        self.as_mut().set_import_text(QString::from(t));
    }

    fn press(&self, t: &QString, down: bool) {
        send(t, ControlValue::Press(down));
    }

    fn set_value(&self, t: &QString, value: f64) {
        send(t, ControlValue::Absolute(value.clamp(0.0, 1.0) as f32));
    }

    fn nudge(&self, t: &QString, delta: f64) {
        send(t, ControlValue::Delta(delta as f32));
    }

    fn load_track(&self, deck: i32, track_id: i64) {
        if let Some(app) = app() {
            app.load_track(deck.clamp(0, 3) as u8, track_id);
        }
    }

    fn load_url(&self, deck: i32, url: &QUrl) {
        if let Some(app) = app() {
            app.load_file(deck.clamp(0, 3) as u8, &url_path(url));
        }
    }

    fn eject(&self, deck: i32) {
        if let Some(app) = app() {
            app.eject(deck.clamp(0, 3) as u8);
        }
    }

    fn grid_edit(&self, deck: i32, op: &QString, arg: f64) {
        let edit = match op.to_string().as_str() {
            "move" => GridEdit::Move(arg),
            "double" => GridEdit::DoubleTempo,
            "halve" => GridEdit::HalveTempo,
            "beat" => GridEdit::BeatHere,
            "downbeat" => GridEdit::DownbeatHere,
            "barstart" => GridEdit::BarStartHere,
            "tap" => GridEdit::Tap,
            "lock" => GridEdit::Lock(arg > 0.5),
            "reset" => GridEdit::Reset,
            _ => return,
        };
        if let Some(app) = app() {
            app.grid_edit(deck.clamp(0, 3) as u8, edit);
        }
    }

    fn set_learn(&self, on: bool) {
        if let Some(app) = app() {
            app.set_learn(on);
        }
    }

    fn add_music_folder(&self, url: &QUrl) {
        if let Some(app) = app() {
            app.add_music_folder(url_path(url));
        }
    }

    fn remove_music_folder(&self, path: &QString) {
        if let Some(app) = app() {
            let mut s = app.settings();
            let p = PathBuf::from(path.to_string());
            s.library_roots.retain(|r| *r != p);
            app.set_settings(s);
        }
    }

    fn rescan(&self) {
        if let Some(app) = app() {
            app.scan();
        }
    }

    fn import_nml(self: Pin<&mut Self>, url: &QUrl) {
        if let Some(app) = app() {
            let text = match app.import_nml(&url_path(url)) {
                Ok(t) => format!("Traktor import: {t}"),
                Err(e) => format!("Traktor import failed: {e}"),
            };
            self.set_status(QString::from(text));
        }
    }

    fn settings_json(&self) -> QString {
        let Some(app) = app() else { return QString::default() };
        let s = app.settings();
        let roots: Vec<String> = s.library_roots.iter().map(|r| json_str(&r.display().to_string())).collect();
        QString::from(format!(
            r#"{{"audio_device":{},"buffer_frames":{},"tempo_range":{},"deck_count":{},"split_cue":{},"bpm_min":{},"bpm_max":{},"key_notation":"{}","auto_gain":{},"target_lufs":{},"library_roots":[{}],"midi":{},"waveform_seconds":{},"waveform_style":"{}","waveform_bottom":{},"waveform_height":{},"waveform_mixer":{},"waveform_fader_dim":{},"background_analysis":{},"header_meter":{},"show_mixer":{},"browser_columns":{},"remix_decks":{},"mixing":"{}","mixer_channels":{},"suggestions":{},"browser_row_size":{},"browser_sidebar_width":{},"beatport_quality":{},"beatport_cache_mb":{}}}"#,
            s.audio_device.as_deref().map_or("null".into(), json_str),
            s.buffer_frames.map_or("null".into(), |b| b.to_string()),
            s.tempo_range,
            s.deck_count,
            s.split_cue,
            s.bpm_min,
            s.bpm_max,
            match s.key_notation {
                KeyNotation::Camelot => "camelot",
                KeyNotation::OpenKey => "open_key",
                KeyNotation::Musical => "musical",
            },
            s.auto_gain,
            s.target_lufs,
            roots.join(","),
            s.midi,
            s.waveform_seconds,
            s.waveform_style.name(),
            s.waveform_bottom,
            s.waveform_height,
            s.waveform_mixer,
            s.waveform_fader_dim,
            s.background_analysis,
            s.header_meter,
            s.show_mixer,
            json_str(&s.browser_columns),
            json_str(&s.remix_decks),
            s.mixing.name(),
            json_str(&s.mixer_channels),
            s.suggestions,
            s.browser_row_size,
            s.browser_sidebar_width,
            json_str(&s.beatport_quality),
            s.beatport_cache_mb
        ))
    }

    fn set_setting(&self, name: &QString, value: &QString) {
        let Some(app) = app() else { return };
        let mut s = app.settings();
        let v = value.to_string();
        let f = v.parse::<f64>().ok();
        let b = v == "true";
        match name.to_string().as_str() {
            "audio_device" => s.audio_device = (!v.is_empty()).then_some(v.clone()),
            "buffer_frames" => s.buffer_frames = v.parse().ok(),
            "tempo_range" => s.tempo_range = f.unwrap_or(s.tempo_range).clamp(0.02, 1.0),
            "deck_count" => s.deck_count = if v == "4" { 4 } else { 2 },
            "split_cue" => s.split_cue = b,
            "mixing" => s.mixing = MixingMode::from_name(&v),
            "mixer_channels" => {
                let mut seen = String::new();
                for c in v.to_uppercase().chars().filter(|c| ('A'..='D').contains(c)) {
                    if !seen.contains(c) {
                        seen.push(c);
                    }
                }
                s.mixer_channels = seen;
            }
            "browser_columns" => s.browser_columns = v.clone(),
            "browser_row_size" => s.browser_row_size = v.parse::<u8>().unwrap_or(0).min(2),
            "browser_sidebar_width" => {
                s.browser_sidebar_width = f.map_or(s.browser_sidebar_width, |w| w.round().clamp(160.0, 800.0) as u16);
            }
            "remix_decks" => s.remix_decks = v.to_uppercase().chars().filter(|c| ('A'..='D').contains(c)).collect(),
            "bpm_min" => s.bpm_min = f.unwrap_or(s.bpm_min),
            "bpm_max" => s.bpm_max = f.unwrap_or(s.bpm_max),
            "key_notation" => {
                s.key_notation = match v.as_str() {
                    "open_key" => KeyNotation::OpenKey,
                    "musical" => KeyNotation::Musical,
                    _ => KeyNotation::Camelot,
                }
            }
            "auto_gain" => s.auto_gain = b,
            "target_lufs" => s.target_lufs = f.unwrap_or(f64::from(s.target_lufs)) as f32,
            "midi" => s.midi = b,
            "waveform_seconds" => s.waveform_seconds = f.unwrap_or(8.0).clamp(2.0, 32.0) as f32,
            "waveform_style" => s.waveform_style = WaveformStyle::from_name(&v),
            "waveform_bottom" => s.waveform_bottom = b,
            "waveform_height" => s.waveform_height = f.unwrap_or(1.0).clamp(0.5, 2.0) as f32,
            "waveform_mixer" => s.waveform_mixer = b,
            "waveform_fader_dim" => s.waveform_fader_dim = b,
            "background_analysis" => s.background_analysis = b,
            "header_meter" => s.header_meter = b,
            "show_mixer" => s.show_mixer = b,
            "suggestions" => s.suggestions = b,
            "beatport_quality" => s.beatport_quality = rille_app::BeatportQuality::from_name(&v).name().to_owned(),
            "beatport_cache_mb" => {
                s.beatport_cache_mb = v.parse::<u32>().unwrap_or(s.beatport_cache_mb).clamp(512, 1 << 20)
            }
            _ => return,
        }
        app.set_settings(s);
    }

    fn audio_devices_json(&self) -> QString {
        let devs = app().map(|a| a.output_devices()).unwrap_or_default();
        QString::from(format!("[{}]", devs.iter().map(|d| json_str(d)).collect::<Vec<_>>().join(",")))
    }

    fn midi_json(&self) -> QString {
        let Some(app) = app() else { return QString::from("{}") };
        let ports: Vec<String> = app.midi_ports().iter().map(|p| json_str(p)).collect();
        let connected: Vec<String> = app
            .midi_devices()
            .iter()
            .map(|(p, m)| {
                format!(r#"{{"port":{},"mapping":{}}}"#, json_str(p), m.as_deref().map_or("null".into(), json_str))
            })
            .collect();
        let maps: Vec<String> = app.mapping_names().iter().map(|m| json_str(m)).collect();
        QString::from(format!(
            r#"{{"ports":[{}],"connected":[{}],"mappings":[{}]}}"#,
            ports.join(","),
            connected.join(","),
            maps.join(",")
        ))
    }

    fn set_port_mapping(&self, port: &QString, mapping: &QString) {
        if let Some(app) = app() {
            let m = mapping.to_string();
            app.set_port_mapping(&port.to_string(), (!m.is_empty()).then_some(m.as_str()));
        }
    }

    fn create_playlist(&self, name: &QString) -> i64 {
        app().and_then(|a| a.create_playlist(&name.to_string())).unwrap_or(-1)
    }

    fn add_to_playlist(&self, playlist: i64, track_id: i64) {
        if let Some(app) = app() {
            app.add_to_playlist(playlist, &[track_id]);
        }
    }

    fn delete_playlist(&self, playlist: i64) {
        if let Some(app) = app() {
            app.delete_playlist(playlist);
        }
    }

    fn set_rating(&self, track_id: i64, stars: i32) {
        if let Some(app) = app() {
            app.set_rating(track_id, stars.clamp(0, 5) as u8);
        }
    }

    fn set_clock_tempo(&self, bpm: f64) {
        if let Some(app) = app() {
            app.command(Command::SetClockBpm(bpm));
        }
    }

    fn set_internal_master(&self) {
        if let Some(app) = app() {
            app.command(Command::SetMaster(None));
        }
    }

    fn effect_names_json(&self) -> QString {
        QString::from(format!("[{}]", fx_names().iter().map(|n| json_str(n)).collect::<Vec<_>>().join(",")))
    }

    fn version(&self) -> QString {
        QString::from(env!("CARGO_PKG_VERSION"))
    }

    fn repository(&self) -> QString {
        QString::from(env!("CARGO_PKG_REPOSITORY"))
    }

    fn smoke_test(&self) -> bool {
        std::env::args().any(|a| a == "--smoke-test")
    }

    fn token_path_text(&self, token: i64) -> QString {
        QString::from(token_path(token).map(|p| p.display().to_string()).unwrap_or_default())
    }

    fn browse_token(&self) -> i64 {
        std::env::args()
            .find_map(|a| a.strip_prefix("--browse=").map(PathBuf::from))
            .map_or(-1, |p| crate::global::path_token(&p))
    }

    fn has_arg(&self, name: &QString) -> bool {
        let flag = format!("--{name}");
        std::env::args().any(|a| a == flag)
    }

    fn track_count(&self) -> i32 {
        app().map_or(0, |a| a.track_count() as i32)
    }

    fn window_size(&self) -> QString {
        QString::from(std::env::args().find_map(|a| a.strip_prefix("--size=").map(str::to_owned)).unwrap_or_default())
    }

    fn screenshot_path(&self) -> QString {
        QString::from(
            std::env::args().find_map(|a| a.strip_prefix("--screenshot=").map(str::to_owned)).unwrap_or_default(),
        )
    }

    fn screenshot_delay(&self) -> i32 {
        let secs = std::env::args().find_map(|a| a.strip_prefix("--delay=").and_then(|s| s.parse::<f64>().ok()));
        secs.map_or(4000, |s| (s.clamp(0.0, 600.0) * 1000.0) as i32)
    }
}

fn fx_names() -> &'static [&'static str] {
    &["None", "Delay", "Reverb", "Filter", "Flanger", "Gater", "Beatmasher"]
}

pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
