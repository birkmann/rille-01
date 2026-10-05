//! User settings (TOML) and the app's directories.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Directory name under the XDG base directories.
pub const APP_DIR: &str = "rille";
/// Where configuration, the library database and caches live.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    /// Recordings of the main mix (created on the first recording).
    pub recordings: PathBuf,
}

impl Paths {
    /// XDG locations: `~/.config/rille`, `~/.local/share/rille`,
    /// `~/.cache/rille`; recordings in the music folder's `rille recordings`.
    pub fn xdg() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let xdg = |var: &str, fallback: &str| {
            let base = std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(fallback));
            base.join(APP_DIR)
        };
        let config = xdg("XDG_CONFIG_HOME", ".config");
        let user_dirs = config.parent().map(|c| c.join("user-dirs.dirs"));
        let music = user_dirs
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| xdg_user_dir(&text, "XDG_MUSIC_DIR", &home))
            .unwrap_or_else(|| home.join("Music"));
        Self {
            config,
            data: xdg("XDG_DATA_HOME", ".local/share"),
            cache: xdg("XDG_CACHE_HOME", ".cache"),
            recordings: music.join("rille recordings"),
        }
    }

    /// Everything under one directory (tests, portable installs).
    pub fn under(root: &Path) -> Self {
        Self {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            recordings: root.join("recordings"),
        }
    }

    pub fn create(&self) -> std::io::Result<()> {
        for d in [&self.config, &self.data, &self.cache, &self.user_mappings()] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.toml")
    }

    pub fn library_db(&self) -> PathBuf {
        self.data.join("library.db")
    }

    /// Moves the library database (with its WAL files) aside to
    /// `library.db.reset-<unix secs>.bak`, so the next start begins with an
    /// empty library. Music files are not touched. Returns the backup.
    pub fn set_library_aside(&self) -> std::io::Result<PathBuf> {
        let db = self.library_db();
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let backup = self.data.join(format!("library.db.reset-{secs}.bak"));
        std::fs::rename(&db, &backup)?;
        for suffix in ["-wal", "-shm"] {
            let side = PathBuf::from(format!("{}{suffix}", db.display()));
            if side.exists() {
                std::fs::rename(&side, format!("{}{suffix}", backup.display()))?;
            }
        }
        Ok(backup)
    }

    pub fn user_mappings(&self) -> PathBuf {
        self.config.join("mappings")
    }

    /// Drum machine: its state and the user's kits (`kits/<name>/`).
    pub fn drums(&self) -> PathBuf {
        self.data.join("drums")
    }

    /// Files of tracks streamed from Beatport.
    pub fn beatport_cache(&self) -> PathBuf {
        self.cache.join("beatport")
    }

    /// The Beatport sign-in (tokens only, never the password).
    pub fn beatport_token(&self) -> PathBuf {
        self.data.join("beatport-token.json")
    }
}

/// A folder from `user-dirs.dirs` (lines like `XDG_MUSIC_DIR="$HOME/Music"`);
/// `None` when it is missing or the home folder itself.
fn xdg_user_dir(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().find_map(|l| l.trim().strip_prefix(key)?.trim_start().strip_prefix('='))?;
    let value = value.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    (path.is_absolute() && path != home).then_some(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeyNotation {
    #[default]
    Camelot,
    OpenKey,
    Musical,
}

/// Where the decks are mixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MixingMode {
    /// External when the output is a hardware mixer's sound card (such as
    /// the Xone:96), else internal.
    #[default]
    Auto,
    /// The on-screen mixer: main mix on outputs 1/2, headphones on 3/4.
    Internal,
    /// Every deck on its own stereo output pair, see
    /// [`Settings::mixer_channels`].
    External,
}

impl MixingMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Internal => "internal",
            Self::External => "external",
        }
    }

    pub fn from_name(name: &str) -> Self {
        [Self::Auto, Self::Internal, Self::External].into_iter().find(|m| m.name() == name).unwrap_or_default()
    }
}

/// How waveform columns are colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaveformStyle {
    /// Bass red, mids green, highs blue, mixed like light; pastel with a
    /// bright high-band core.
    Spectrum,
    /// Bands drawn on top of each other: lows blue, mids amber, highs white
    /// (the club media player look).
    #[default]
    ThreeBand,
    /// One saturated color per column from the band mix.
    Rgb,
    /// Single blue, brighter where highs dominate.
    Mono,
}

impl WaveformStyle {
    pub const ALL: [Self; 4] = [Self::Spectrum, Self::ThreeBand, Self::Rgb, Self::Mono];

    pub fn name(self) -> &'static str {
        match self {
            Self::Spectrum => "spectrum",
            Self::ThreeBand => "three_band",
            Self::Rgb => "rgb",
            Self::Mono => "mono",
        }
    }

    pub fn from_name(name: &str) -> Self {
        Self::ALL.into_iter().find(|s| s.name() == name).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Output device; `None` = system default.
    pub audio_device: Option<String>,
    /// Buffer size in frames; `None` = device default.
    pub buffer_frames: Option<u32>,
    /// Tempo fader range (0.08 = ±8 %).
    pub tempo_range: f64,
    /// Decks on screen: 2 (A, B) or 4 (A/C left, B/D right, mixer C A B D).
    pub deck_count: u8,
    /// Height of all decks, the extra room going to the waveforms: 0 normal,
    /// 1 tall, 2 taller.
    pub deck_height: u8,
    /// The browser's hover load buttons with 4 decks: true in two rows
    /// like the decks on screen (A B over C D), false in one row (A B C D).
    pub load_buttons_grid: bool,
    /// Letters of the decks that are remix decks, e.g. "CD".
    pub remix_decks: String,
    /// The drum machine panel is shown.
    pub drums_visible: bool,
    /// Where the drum machine panel sits: 0 above the decks, 1 below.
    pub drums_position: u8,
    /// Sequencer rows shown at once: 1 (the selected instrument) or 4.
    pub drums_rows: u8,
    /// Headphones on the left channel, master on the right (2-channel cards).
    pub split_cue: bool,
    pub mixing: MixingMode,
    /// External mixing: the deck on each mixer channel, left to right, e.g.
    /// "CABD" = deck C on outputs 1/2 (Xone:96 channel 1), A on 3/4, B on
    /// 5/6, D on 7/8.
    pub mixer_channels: String,
    /// Analysis folds tempos into this range.
    pub bpm_min: f64,
    pub bpm_max: f64,
    pub key_notation: KeyNotation,
    /// Level tracks to `target_lufs` with the channel gain.
    pub auto_gain: bool,
    pub target_lufs: f32,
    /// Music folders scanned into the collection.
    pub library_roots: Vec<PathBuf>,
    pub midi: bool,
    /// Seconds of audio visible in the scrolling waveform.
    pub waveform_seconds: f32,
    pub waveform_style: WaveformStyle,
    /// Waveforms grow up from the bottom edge instead of around the centre.
    pub waveform_bottom: bool,
    /// Vertical scale of the waveforms (1 = as analyzed).
    pub waveform_height: f32,
    /// The scrolling waveform shows the channel's GAIN, EQ and filter.
    pub waveform_mixer: bool,
    /// The scrolling waveform dims with the channel fader and crossfader.
    pub waveform_fader_dim: bool,
    /// Analyze new tracks in the background. Cancelling the analysis turns
    /// it off.
    pub background_analysis: bool,
    /// The analysis queue is paused (kept over restarts until resumed).
    pub analysis_paused: bool,
    /// Main output meter in the title bar.
    pub header_meter: bool,
    /// The on-screen mixer and crossfader; off leaves the room to the decks
    /// when a controller has the knobs and faders.
    pub show_mixer: bool,
    /// A KEY (key shift) knob on each mixer channel.
    pub mixer_key: bool,
    /// Refuse to load a track onto a deck that is on air: playing with its
    /// channel fader at `load_lock_level` or above and the crossfader not
    /// cutting it.
    pub load_lock: bool,
    /// Channel fader position (`0..1`) from which a playing deck counts as
    /// on air.
    pub load_lock_level: f32,
    /// Letters of the decks the load lock protects, e.g. "ABCD".
    pub load_lock_decks: String,
    /// Mapping chosen per MIDI device (port name without the ALSA
    /// `client:port` numbers); an empty name means no mapping. Other devices
    /// get the first mapping whose device pattern matches.
    pub midi_mappings: BTreeMap<String, String>,
    /// MIDI channel (`1..=16`) per controller whose mapping names its
    /// factory channel (by mapping name as written in its file), for a
    /// controller set to another channel.
    pub midi_channels: BTreeMap<String, u8>,
    /// The browser's track columns as JSON (`[{"key":…,"width":…}]` in
    /// display order); empty = the default layout.
    pub browser_columns: String,
    /// Height of the browser's track rows and their covers: 0 compact,
    /// 1 medium, 2 large.
    pub browser_row_size: u8,
    /// Width of the browser's source tree (left column) in pixels.
    pub browser_sidebar_width: u16,
    /// The browser lists tracks that fit the playing one (tempo, key,
    /// genre) under "Suggestions".
    pub suggestions: bool,
    /// Beatport streaming quality: "lossless" (FLAC), "high" (AAC 256) or
    /// "medium" (AAC 128).
    pub beatport_quality: String,
    /// Streamed tracks kept on disk, in megabytes; the least recently
    /// played go first, never those downloaded for offline use.
    pub beatport_cache_mb: u32,
    /// Separated stems kept on disk, in megabytes (a 6-minute track takes
    /// about 190); the least recently used go first.
    pub stems_cache_mb: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            audio_device: None,
            buffer_frames: Some(256),
            tempo_range: 0.08,
            deck_count: 2,
            deck_height: 0,
            load_buttons_grid: true,
            remix_decks: String::new(),
            drums_visible: false,
            drums_position: 0,
            drums_rows: 1,
            split_cue: false,
            mixing: MixingMode::Auto,
            mixer_channels: "CABD".into(),
            bpm_min: 88.0,
            bpm_max: 175.0,
            key_notation: KeyNotation::Camelot,
            auto_gain: true,
            target_lufs: -10.0,
            library_roots: Vec::new(),
            midi: true,
            waveform_seconds: 8.0,
            waveform_style: WaveformStyle::ThreeBand,
            waveform_bottom: false,
            waveform_height: 1.0,
            waveform_mixer: true,
            waveform_fader_dim: false,
            background_analysis: true,
            analysis_paused: false,
            header_meter: false,
            show_mixer: true,
            mixer_key: false,
            load_lock: true,
            load_lock_level: 0.5,
            load_lock_decks: "ABCD".into(),
            midi_mappings: BTreeMap::new(),
            midi_channels: BTreeMap::new(),
            browser_columns: String::new(),
            browser_row_size: 0,
            browser_sidebar_width: 250,
            suggestions: false,
            beatport_quality: "lossless".into(),
            beatport_cache_mb: 20 * 1024,
            stems_cache_mb: 10 * 1024,
        }
    }
}

impl Settings {
    /// Loads settings, falling back to defaults for a missing or broken file.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// External mixing: the stereo output pair of each deck (`None` for a
    /// deck on no mixer channel), from [`Self::mixer_channels`].
    pub fn deck_outputs(&self) -> [Option<u8>; rille_engine::MAX_DECKS] {
        let mut out = [None; rille_engine::MAX_DECKS];
        for (pair, c) in self.mixer_channels.chars().enumerate() {
            let d = (c.to_ascii_uppercase() as usize).wrapping_sub('A' as usize);
            if let Some(slot) = out.get_mut(d).filter(|s| s.is_none()) {
                *slot = u8::try_from(pair).ok();
            }
        }
        out
    }

    pub fn key_text(&self, key: rille_core::Key) -> String {
        match self.key_notation {
            KeyNotation::Camelot => key.camelot(),
            KeyNotation::OpenKey => key.open_key(),
            KeyNotation::Musical => key.musical(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_folder_from_user_dirs() {
        let home = Path::new("/home/u");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_MUSIC_DIR=\"$HOME/Musik\"\n";
        assert_eq!(xdg_user_dir(text, "XDG_MUSIC_DIR", home), Some(PathBuf::from("/home/u/Musik")));
        assert_eq!(xdg_user_dir("XDG_MUSIC_DIR=\"/data/music\"", "XDG_MUSIC_DIR", home), Some("/data/music".into()));
        assert_eq!(xdg_user_dir("XDG_MUSIC_DIR=\"$HOME/\"", "XDG_MUSIC_DIR", home), None);
        assert_eq!(xdg_user_dir(text, "XDG_VIDEOS_DIR", home), None);
    }

    #[test]
    fn library_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.create().unwrap();
        std::fs::write(paths.library_db(), "db").unwrap();
        std::fs::write(paths.data.join("library.db-wal"), "wal").unwrap();
        let backup = paths.set_library_aside().unwrap();
        assert!(!paths.library_db().exists());
        assert!(!paths.data.join("library.db-wal").exists());
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "db");
        assert_eq!(std::fs::read_to_string(format!("{}-wal", backup.display())).unwrap(), "wal");
    }

    #[test]
    fn roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.toml");
        assert_eq!(Settings::load(&p), Settings::default());
        let midi_mappings = [("XONE:K2:XONE:K2 MIDI 1".to_string(), "Allen & Heath Xone:K2 (ABCD)".to_string())];
        let s = Settings {
            tempo_range: 0.16,
            library_roots: vec!["/music".into()],
            midi_mappings: midi_mappings.into(),
            waveform_style: WaveformStyle::Rgb,
            waveform_mixer: false,
            show_mixer: false,
            deck_count: 4,
            deck_height: 2,
            drums_visible: true,
            drums_position: 1,
            drums_rows: 4,
            ..Settings::default()
        };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
        // Unknown or missing fields fall back to defaults.
        std::fs::write(&p, "tempo_range = 0.5\nbogus = 1\n").unwrap();
        assert_eq!(Settings::load(&p).tempo_range, 0.5);
        assert!(Settings::load(&p).show_mixer, "older files keep the mixer");
        assert!(!Settings::load(&p).suggestions, "suggestions are opt-in");
    }

    #[test]
    fn deck_outputs_follow_the_mixer_channels() {
        let s = Settings::default();
        assert_eq!(s.mixing, MixingMode::Auto);
        assert_eq!(s.deck_outputs(), [Some(1), Some(2), Some(0), Some(3)], "C A B D on channels 1-4");
        let s = Settings { mixer_channels: "ab".into(), ..Settings::default() };
        assert_eq!(s.deck_outputs(), [Some(0), Some(1), None, None]);
        let s = Settings { mixer_channels: "AAXB".into(), ..Settings::default() };
        assert_eq!(s.deck_outputs(), [Some(0), Some(3), None, None], "first channel wins, unknown letters skipped");
        assert_eq!(MixingMode::from_name("external"), MixingMode::External);
        assert_eq!(MixingMode::from_name("bogus"), MixingMode::Auto);
    }
}
