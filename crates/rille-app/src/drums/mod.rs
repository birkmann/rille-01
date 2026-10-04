//! The drum machine on the app side: kits (loading, making your own),
//! saving and restoring patterns and settings, and the controls the engine
//! cannot handle alone.
//!
//! The engine holds the live state (it is edited from controllers too); the
//! app saves it to `<data>/drums/state.toml` a moment after it changes and
//! puts it back into a new engine.

pub mod kits;
pub mod synth;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use rille_core::drums::{INSTRUMENTS, NAMES, PATTERNS, Pattern, factory_patterns};
use rille_engine::{Command, DrumKit, DrumParams, DrumState, TrackAudio};
use rille_library::TrackId;
use serde::{Deserialize, Serialize};

use crate::{App, UiEvent};
pub use kits::KitInfo;

/// Edits are saved once the state has been still this long.
const SAVE_AFTER: Duration = Duration::from_secs(1);

/// Everything about the drum machine that is kept: kit, patterns, levels.
#[derive(Clone, Debug, PartialEq)]
pub struct DrumSet {
    pub kit: String,
    pub patterns: [Pattern; PATTERNS],
    pub params: DrumParams,
}

impl Default for DrumSet {
    fn default() -> Self {
        Self { kit: synth::FACTORY_KITS[0].into(), patterns: factory_patterns(), params: DrumParams::default() }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct SetFile {
    kit: String,
    current: u8,
    selected: u8,
    level: f32,
    filter: f32,
    fx: [bool; 2],
    instruments: Vec<InstFile>,
    patterns: Vec<PatternFile>,
}

impl Default for SetFile {
    fn default() -> Self {
        let p = DrumParams::default();
        Self {
            kit: synth::FACTORY_KITS[0].into(),
            current: 0,
            selected: 0,
            level: p.channel_level,
            filter: p.filter,
            fx: p.fx_assign,
            instruments: Vec::new(),
            patterns: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct InstFile {
    name: String,
    level: f32,
    tune: f32,
    decay: f32,
    muted: bool,
}

impl Default for InstFile {
    fn default() -> Self {
        Self { name: String::new(), level: 1.0, tune: 0.5, decay: 1.0, muted: false }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct PatternFile {
    length: u8,
    swing: f32,
    /// Instrument name → steps as text (`x...X...`, see `Pattern::row_text`).
    rows: BTreeMap<String, String>,
}

impl Default for PatternFile {
    fn default() -> Self {
        Self { length: 16, swing: 0.0, rows: BTreeMap::new() }
    }
}

impl DrumSet {
    /// The set as the engine has it now, with kit `kit`.
    pub fn from_state(kit: &str, s: &DrumState) -> Self {
        Self {
            kit: kit.to_owned(),
            patterns: s.patterns,
            params: DrumParams {
                level: s.inst.map(|i| i.level),
                tune: s.inst.map(|i| i.tune),
                decay: s.inst.map(|i| i.decay),
                muted: s.inst.map(|i| i.muted),
                // A pattern waiting to start is where the user is.
                current: s.queued.unwrap_or(s.current),
                selected: s.selected,
                channel_level: s.channel.volume,
                filter: s.channel.filter,
                fx_assign: s.channel.fx_assign,
            },
        }
    }

    pub fn load(path: &Path) -> Option<Self> {
        let f: SetFile = toml::from_str(&std::fs::read_to_string(path).ok()?)
            .map_err(|e| eprintln!("{}: {e}", path.display()))
            .ok()?;
        let mut params = DrumParams {
            current: f.current.min(PATTERNS as u8 - 1),
            selected: f.selected.min(INSTRUMENTS as u8 - 1),
            channel_level: f.level.clamp(0.0, 1.0),
            filter: f.filter.clamp(0.0, 1.0),
            fx_assign: f.fx,
            ..DrumParams::default()
        };
        for inst in &f.instruments {
            if let Some(i) = NAMES.iter().position(|n| n.eq_ignore_ascii_case(&inst.name)) {
                params.level[i] = inst.level.clamp(0.0, 1.0);
                params.tune[i] = inst.tune.clamp(0.0, 1.0);
                params.decay[i] = inst.decay.clamp(0.0, 1.0);
                params.muted[i] = inst.muted;
            }
        }
        let mut patterns = [Pattern::default(); PATTERNS];
        for (p, pf) in patterns.iter_mut().zip(&f.patterns) {
            p.length = pf.length.clamp(1, 16);
            p.swing = pf.swing.clamp(0.0, 1.0);
            for (name, text) in &pf.rows {
                if let Some(i) = NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)) {
                    p.set_row_text(i, text);
                }
            }
        }
        Some(Self { kit: f.kit, patterns, params })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let p = &self.params;
        let file = SetFile {
            kit: self.kit.clone(),
            current: p.current,
            selected: p.selected,
            level: p.channel_level,
            filter: p.filter,
            fx: p.fx_assign,
            instruments: (0..INSTRUMENTS)
                .map(|i| InstFile {
                    name: NAMES[i].into(),
                    level: p.level[i],
                    tune: p.tune[i],
                    decay: p.decay[i],
                    muted: p.muted[i],
                })
                .collect(),
            patterns: self
                .patterns
                .iter()
                .map(|pat| PatternFile {
                    length: pat.length,
                    swing: pat.swing,
                    rows: (0..INSTRUMENTS)
                        .filter(|&i| pat.has_steps(i))
                        .map(|i| (NAMES[i].to_owned(), pat.row_text(i)))
                        .collect(),
                })
                .collect(),
        };
        let text = toml::to_string_pretty(&file).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

/// The app's drum machine state.
pub(crate) struct Drums {
    /// What is saved and what a new engine gets.
    set: DrumSet,
    /// The kit's samples as sent to the engine (for "new kit from current").
    samples: kits::Samples,
    /// Engine edit revision seen last (`None` before an engine got the
    /// set), when it changed, and whether that change is saved.
    seen_rev: Option<u32>,
    changed_at: Instant,
    dirty: bool,
    /// Kit loads started; a slower, older load does not win.
    load_seq: u64,
    clipboard: Option<Pattern>,
}

impl Drums {
    pub(crate) fn load(drums_dir: &Path) -> Self {
        Self {
            set: DrumSet::load(&state_file(drums_dir)).unwrap_or_default(),
            samples: Default::default(),
            seen_rev: None,
            changed_at: Instant::now(),
            dirty: false,
            load_seq: 0,
            clipboard: None,
        }
    }
}

fn state_file(drums_dir: &Path) -> PathBuf {
    drums_dir.join("state.toml")
}

impl App {
    fn drums_lock(&self) -> std::sync::MutexGuard<'_, Drums> {
        self.drums.lock().expect("drums lock")
    }

    /// Gives a new engine the drum machine as it was: patterns and settings
    /// at once, the kit when its samples are ready.
    pub(crate) fn install_drums(self: &Arc<Self>) {
        let set = self.drums_lock().set.clone();
        for (i, p) in set.patterns.iter().enumerate() {
            self.send(Command::SetDrumPattern { index: i as u8, pattern: *p, undo: false });
        }
        self.send(Command::SetDrumParams(set.params));
        // A new engine starts at revision 0; restoring counts as an edit
        // (saving the same again), and nothing after it can be missed.
        self.drums_lock().seen_rev = Some(0);
        let samples = self.drums_lock().samples.clone();
        if samples.iter().any(Option::is_some) {
            self.send(Command::SetDrumKit(Some(Arc::new(DrumKit { samples }))));
        } else {
            self.select_drum_kit(&set.kit);
        }
    }

    /// Takes the engine's drum state into the saved set (before the engine
    /// goes away, and when quitting).
    pub(crate) fn capture_drums(&self) {
        let Some(engine) = self.engine.get() else { return };
        let state = engine.snapshot().drums;
        let mut d = self.drums_lock();
        if d.seen_rev.is_none() || state.edit_rev == 0 {
            // The engine has not shown the restored set yet.
            return;
        }
        d.set = DrumSet::from_state(&d.set.kit, &state);
        d.dirty = false;
        self.save_drum_set(&d);
    }

    /// Saves the drum machine a moment after it was edited.
    pub(crate) fn drums_tick(&self, s: &DrumState) {
        let mut d = self.drums_lock();
        let now = Instant::now();
        match d.seen_rev {
            Some(rev) if rev != s.edit_rev => {
                d.seen_rev = Some(s.edit_rev);
                d.changed_at = now;
                d.dirty = true;
            }
            // No engine set up yet.
            Some(_) | None => {}
        }
        if d.dirty && now.duration_since(d.changed_at) >= SAVE_AFTER {
            d.set = DrumSet::from_state(&d.set.kit, s);
            d.dirty = false;
            self.save_drum_set(&d);
        }
    }

    fn save_drum_set(&self, d: &Drums) {
        if let Err(e) = d.set.save(&state_file(&self.paths.drums())) {
            eprintln!("saving the drum machine: {e}");
        }
    }

    pub fn drum_kits(&self) -> Vec<KitInfo> {
        kits::list(&self.paths.drums())
    }

    pub fn drum_kit_name(&self) -> String {
        self.drums_lock().set.kit.clone()
    }

    /// Where the user's kits live (created on demand).
    pub fn drum_kits_folder(&self) -> PathBuf {
        let dir = kits::kits_dir(&self.paths.drums());
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Switches to kit `name`; its samples load in the background.
    pub fn select_drum_kit(self: &Arc<Self>, name: &str) {
        let seq = {
            let mut d = self.drums_lock();
            d.load_seq += 1;
            d.load_seq
        };
        let (app, name) = (self.clone(), name.to_owned());
        std::thread::Builder::new()
            .name("drum-kit".into())
            .spawn(move || {
                let (name, samples) = match kits::load(&app.paths.drums(), &name) {
                    Ok(s) => (name, s),
                    Err(e) => {
                        app.notify(UiEvent::Status(format!("{e}; playing the factory kit")));
                        let factory = synth::FACTORY_KITS[0].to_owned();
                        let s = kits::load(&app.paths.drums(), &factory).unwrap_or_default();
                        (factory, s)
                    }
                };
                let mut d = app.drums_lock();
                if d.load_seq != seq {
                    return;
                }
                d.samples = samples.clone();
                if d.set.kit != name {
                    d.set.kit = name;
                    app.save_drum_set(&d);
                }
                drop(d);
                app.send(Command::SetDrumKit(Some(Arc::new(DrumKit { samples }))));
                app.refresh_drum_kit_list();
                app.notify(UiEvent::DrumsChanged);
            })
            .expect("spawn drum kit loader");
    }

    /// Next or previous kit (`steps` > 0 or < 0).
    pub(crate) fn step_drum_kit(self: &Arc<Self>, steps: i64) {
        let kits = self.drum_kits();
        let current = self.drum_kit_name();
        let i = kits.iter().position(|k| k.name == current).unwrap_or(0) as i64;
        let next = (i + steps).rem_euclid(kits.len() as i64) as usize;
        self.select_drum_kit(&kits[next].name);
    }

    /// Kit `index` of the list (factory kits first), if there is one.
    pub(crate) fn pick_drum_kit(self: &Arc<Self>, index: usize) {
        if let Some(k) = self.drum_kits().get(index) {
            self.select_drum_kit(&k.name);
        }
    }

    /// Reads the kit list again for controllers (see [`crate::values::KitList`]).
    pub(crate) fn refresh_drum_kit_list(&self) {
        let names: Vec<String> = self.drum_kits().into_iter().map(|k| k.name).collect();
        let current = self.drum_kit_name();
        let current = names.iter().position(|n| *n == current).unwrap_or(0);
        *self.drum_kit_list.write().expect("kit list lock") = crate::values::KitList { names, current };
    }

    /// State controllers read besides the engine's.
    pub(crate) fn app_values(&self) -> crate::values::AppValues {
        crate::values::AppValues {
            recording: self.recording_on.clone(),
            drums_visible: self.drums_visible.clone(),
            kits: self.drum_kit_list.clone(),
        }
    }

    /// Shows the drum machine when a drum controller (a mapping with a drum
    /// screen) connects.
    pub(crate) fn show_drums_for(&self, mapping: Option<&str>) {
        let Some(name) = mapping else { return };
        let is_drum = {
            let store = self.mappings.lock().expect("mappings lock");
            store
                .by_name(name)
                .and_then(|m| m.hid.as_ref())
                .and_then(|h| h.bitmap.as_ref())
                .is_some_and(|b| b.screen == rille_midi::screen::ScreenKind::Drum)
        };
        if is_drum && !self.drums_visible() {
            self.toggle_drums_visible();
        }
    }

    /// Saves the current kit's samples as a new kit of your own, `name`,
    /// and switches to it. Returns its name.
    pub fn new_drum_kit(&self, name: &str) -> Result<String, String> {
        let drums = self.paths.drums();
        let mut d = self.drums_lock();
        let name = kits::create(&drums, name, &d.samples)?;
        d.set.kit = name.clone();
        self.save_drum_set(&d);
        drop(d);
        self.refresh_drum_kit_list();
        self.notify(UiEvent::DrumsChanged);
        Ok(name)
    }

    /// The current kit when it is the user's, else a new copy of it (the
    /// factory kits stay as they are).
    fn own_drum_kit(&self) -> Result<String, String> {
        let current = self.drum_kit_name();
        if !kits::is_factory(&current) {
            return Ok(current);
        }
        let name = kits::unique_name(&self.paths.drums(), &format!("My {current}"));
        let name = self.new_drum_kit(&name)?;
        self.notify(UiEvent::Status(format!("Made the kit \"{name}\" from {current}, to change its sounds")));
        Ok(name)
    }

    /// Puts `audio` into instrument `inst` of the user's kit.
    fn set_drum_sample(&self, inst: usize, audio: Option<TrackAudio>) -> Result<(), String> {
        let kit = self.own_drum_kit()?;
        let drums = self.paths.drums();
        match &audio {
            Some(a) => kits::set_sample(&drums, &kit, inst, a)?,
            None => kits::remove_sample(&drums, &kit, inst)?,
        }
        let mut d = self.drums_lock();
        d.samples[inst] = audio.map(Arc::new);
        let samples = d.samples.clone();
        drop(d);
        self.send(Command::SetDrumKit(Some(Arc::new(DrumKit { samples }))));
        self.notify(UiEvent::DrumsChanged);
        Ok(())
    }

    /// Loads an audio file as instrument `inst`'s sound (in the background).
    pub fn drum_load_file(self: &Arc<Self>, inst: usize, path: &Path) {
        if inst >= INSTRUMENTS {
            return;
        }
        let (app, path) = (self.clone(), path.to_owned());
        std::thread::Builder::new()
            .name("drum-sample".into())
            .spawn(move || {
                let r = kits::decode(&path).and_then(|a| app.set_drum_sample(inst, Some(a)));
                if let Err(e) = r {
                    app.notify(UiEvent::Status(format!("Cannot load {}: {e}", path.display())));
                }
            })
            .expect("spawn drum sample loader");
    }

    /// Loads a library track as instrument `inst`'s sound (its first
    /// seconds; for one-shots in the collection).
    pub fn drum_load_track(self: &Arc<Self>, inst: usize, id: TrackId) {
        let Ok(Some(row)) = self.library.lock().expect("library lock").track(id) else { return };
        let app = self.clone();
        std::thread::Builder::new()
            .name("drum-sample".into())
            .spawn(move || match app.streamed_file(&row, None, None) {
                Ok(p) => app.drum_load_file(inst, &p),
                Err(e) => app.notify(UiEvent::Status(format!("Cannot stream {}: {e}", row.title))),
            })
            .expect("spawn drum sample loader");
    }

    /// Removes instrument `inst`'s sound from the user's kit.
    pub fn drum_clear_sample(&self, inst: usize) {
        if inst < INSTRUMENTS
            && let Err(e) = self.set_drum_sample(inst, None)
        {
            self.notify(UiEvent::Status(e));
        }
    }

    pub fn drum_copy_pattern(&self) {
        let s = self.snapshot().drums;
        self.drums_lock().clipboard = Some(s.patterns[usize::from(s.current)]);
    }

    /// Pastes the copied pattern over the current one.
    pub fn drum_paste_pattern(&self) {
        let Some(p) = self.drums_lock().clipboard else { return };
        let current = self.snapshot().drums.current;
        self.send(Command::SetDrumPattern { index: current, pattern: p, undo: true });
    }

    pub fn drum_has_clipboard(&self) -> bool {
        self.drums_lock().clipboard.is_some()
    }

    pub fn drums_visible(&self) -> bool {
        self.drums_visible.load(Ordering::Relaxed)
    }

    /// Shows or hides the drum machine panel (saved in the settings).
    pub(crate) fn toggle_drums_visible(&self) {
        let mut s = self.settings.write().expect("settings lock");
        s.drums_visible = !s.drums_visible;
        self.drums_visible.store(s.drums_visible, Ordering::Relaxed);
        let _ = s.save(&self.paths.settings_file());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drums/state.toml");
        assert_eq!(DrumSet::load(&path), None);
        let mut set = DrumSet { kit: "Mine".into(), ..DrumSet::default() };
        set.patterns[3].set_accent(5, 7, true);
        set.patterns[3].length = 12;
        set.patterns[3].swing = 0.25;
        set.params.tune[2] = 0.75;
        set.params.muted[7] = true;
        set.params.current = 3;
        set.params.fx_assign = [false, true];
        set.save(&path).unwrap();
        assert_eq!(DrumSet::load(&path), Some(set));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("BD = \"X...x...X...x...\""), "readable patterns:\n{text}");
    }

    #[test]
    fn broken_or_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, "kit = 3").unwrap();
        assert_eq!(DrumSet::load(&path), None);
        std::fs::write(&path, "current = 99\n[[patterns]]\nrows = { sd = \"x\" }\n").unwrap();
        let set = DrumSet::load(&path).unwrap();
        assert_eq!(set.params.current, 15);
        assert!(set.patterns[0].is_on(1, 0) && set.patterns[0].length == 16);
        assert_eq!(set.params.level, [1.0; INSTRUMENTS]);
    }
}
