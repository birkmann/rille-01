//! The drum machine on the app side: kits (loading, making your own, saving
//! and exporting them as racks), the track order, saving and restoring
//! patterns and settings, and the controls the engine cannot handle alone.
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

use rille_core::drums::{
    DEFAULT_ORDER, INSTRUMENTS, NAMES, Order, PATTERNS, Pattern, factory_patterns, move_track, order_from_names,
    order_names,
};
use rille_core::{Control, ControlEvent, ControlTarget, ControlValue};
use rille_engine::{Command, DrumKit, DrumParams, DrumState, TrackAudio};
use rille_library::TrackId;
use serde::{Deserialize, Serialize};

use crate::{App, UiEvent};
pub use kits::{KitInfo, Labels, Sound};

/// Edits are saved once the state has been still this long.
const SAVE_AFTER: Duration = Duration::from_secs(1);

/// Everything about the drum machine that is kept: kit, track order,
/// patterns, levels.
#[derive(Clone, Debug, PartialEq)]
pub struct DrumSet {
    pub kit: String,
    pub order: Order,
    pub patterns: [Pattern; PATTERNS],
    pub params: DrumParams,
}

impl Default for DrumSet {
    fn default() -> Self {
        Self {
            kit: synth::FACTORY_KITS[0].into(),
            order: DEFAULT_ORDER,
            patterns: factory_patterns(),
            params: DrumParams::default(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct SetFile {
    kit: String,
    /// Instrument names, left to right.
    order: Vec<String>,
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
            order: Vec::new(),
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
    /// This set's kit and order with the engine's state now.
    pub fn with_state(&self, s: &DrumState) -> Self {
        Self {
            kit: self.kit.clone(),
            order: self.order,
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
        Some(Self { kit: f.kit, order: order_from_names(&f.order), patterns, params })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let p = &self.params;
        let file = SetFile {
            kit: self.kit.clone(),
            order: order_names(&self.order),
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
    /// The kit as sent to the engine, and its sounds' names (for "new kit
    /// from current").
    kit: kits::Kit,
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
            kit: Default::default(),
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
        let samples = self.drums_lock().kit.samples.clone();
        if samples.iter().any(Option::is_some) {
            self.send(Command::SetDrumKit(Some(Arc::new(DrumKit { samples }))));
        } else {
            // As it was left: not the order and sounds saved with the kit.
            self.load_drum_kit(&set.kit, false);
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
        d.set = d.set.with_state(&state);
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
            d.set = d.set.with_state(s);
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

    /// Switches to kit `name`, with the track order and sounds saved with it
    /// as a rack; its samples load in the background.
    pub fn select_drum_kit(self: &Arc<Self>, name: &str) {
        self.load_drum_kit(name, true);
    }

    /// Switches to kit `name`; `rack`: also take the order and sounds saved
    /// with it.
    fn load_drum_kit(self: &Arc<Self>, name: &str, rack: bool) {
        let seq = {
            let mut d = self.drums_lock();
            d.load_seq += 1;
            d.load_seq
        };
        let (app, name) = (self.clone(), name.to_owned());
        std::thread::Builder::new()
            .name("drum-kit".into())
            .spawn(move || {
                let (name, kit) = match kits::load(&app.paths.drums(), &name) {
                    Ok(k) => (name, k),
                    Err(e) => {
                        app.notify(UiEvent::Status(format!("{e}; playing the factory kit")));
                        let factory = synth::FACTORY_KITS[0].to_owned();
                        let k = kits::load(&app.paths.drums(), &factory).unwrap_or_default();
                        (factory, k)
                    }
                };
                let mut d = app.drums_lock();
                if d.load_seq != seq {
                    return;
                }
                let samples = kit.samples.clone();
                let sounds = kit.sounds.filter(|_| rack);
                let order = kit.order.filter(|o| rack && *o != d.set.order);
                d.kit = kit;
                if d.set.kit != name || order.is_some() {
                    d.set.kit = name;
                    d.set.order = order.unwrap_or(d.set.order);
                    app.save_drum_set(&d);
                }
                drop(d);
                app.send(Command::SetDrumKit(Some(Arc::new(DrumKit { samples }))));
                if let Some(s) = sounds {
                    app.set_drum_sounds(&s);
                }
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

    /// The instruments' level, tune and decay now.
    fn drum_sounds(&self) -> [Sound; INSTRUMENTS] {
        self.snapshot().drums.inst.map(|i| Sound { level: i.level, tune: i.tune, decay: i.decay })
    }

    /// Gives the instruments these levels, tunes and decays.
    fn set_drum_sounds(&self, sounds: &[Sound; INSTRUMENTS]) {
        for (i, s) in sounds.iter().enumerate() {
            let n = i as u8 + 1;
            for (control, x) in [
                (Control::DrumInstLevel(n), s.level),
                (Control::DrumInstTune(n), s.tune),
                (Control::DrumInstDecay(n), s.decay),
            ] {
                let target = ControlTarget::drum(control);
                self.send(Command::Control(ControlEvent { target, value: ControlValue::Absolute(x) }));
            }
        }
    }

    /// The rack as it is now: the kit's samples, their names (a factory
    /// kit's are called after it), the track order and the sounds.
    fn drum_rack(&self, d: &Drums, sounds: [Sound; INSTRUMENTS]) -> kits::Kit {
        let factory = kits::is_factory(&d.set.kit);
        let labels = std::array::from_fn(|i| match &d.kit.labels[i] {
            Some(l) => Some(l.clone()),
            None if factory && d.kit.samples[i].is_some() => Some(d.set.kit.clone()),
            None => None,
        });
        kits::Kit { samples: d.kit.samples.clone(), labels, order: Some(d.set.order), sounds: Some(sounds) }
    }

    /// Saves the rack (the current kit's samples, the track order and the
    /// sounds) as a new kit of your own, `name`, and switches to it. Returns
    /// its name.
    pub fn new_drum_kit(&self, name: &str) -> Result<String, String> {
        let sounds = self.drum_sounds();
        let drums = self.paths.drums();
        let mut d = self.drums_lock();
        let rack = self.drum_rack(&d, sounds);
        let name = kits::create(&drums, name, &rack)?;
        d.kit.labels = rack.labels;
        d.set.kit = name.clone();
        self.save_drum_set(&d);
        drop(d);
        self.refresh_drum_kit_list();
        self.notify(UiEvent::DrumsChanged);
        Ok(name)
    }

    /// Saves the track order and the sounds with the current kit (one of
    /// the user's; a factory kit is saved under a new name instead).
    pub fn save_drum_rack(&self) -> Result<(), String> {
        let sounds = self.drum_sounds();
        let d = self.drums_lock();
        let r = if kits::is_factory(&d.set.kit) {
            Err(format!("{} is a factory kit: save the rack under a name of your own", d.set.kit))
        } else {
            kits::save_rack(&self.paths.drums(), &d.set.kit, &d.set.order, &sounds)
        };
        let text = match &r {
            Ok(()) => format!("Saved the rack \"{}\"", d.set.kit),
            Err(e) => format!("Cannot save the rack: {e}"),
        };
        self.notify(UiEvent::Status(text));
        r
    }

    /// Writes the rack as it is now into a folder of its own in `dest`, to
    /// share it or keep it outside rille (in the background).
    pub fn export_drum_rack(self: &Arc<Self>, dest: &Path) {
        let sounds = self.drum_sounds();
        let (name, rack) = {
            let d = self.drums_lock();
            (d.set.kit.clone(), self.drum_rack(&d, sounds))
        };
        let (app, dest) = (self.clone(), dest.to_owned());
        std::thread::Builder::new()
            .name("drum-export".into())
            .spawn(move || {
                let text = match kits::export(&dest, &name, &rack) {
                    Ok(dir) => format!("Exported the rack to {}", dir.display()),
                    Err(e) => format!("Cannot export the rack: {e}"),
                };
                app.notify(UiEvent::Status(text));
            })
            .expect("spawn drum rack export");
    }

    /// Copies an exported rack (or a folder of sounds named after the
    /// instruments) to the user's kits and switches to it.
    pub fn import_drum_rack(self: &Arc<Self>, src: &Path) {
        let (app, src) = (self.clone(), src.to_owned());
        std::thread::Builder::new()
            .name("drum-import".into())
            .spawn(move || match kits::import(&app.paths.drums(), &src) {
                Ok(name) => {
                    app.select_drum_kit(&name);
                    app.notify(UiEvent::Status(format!("Imported the rack \"{name}\"")));
                }
                Err(e) => app.notify(UiEvent::Status(format!("Cannot import the rack: {e}"))),
            })
            .expect("spawn drum rack import");
    }

    /// Instruments left to right.
    pub fn drum_order(&self) -> Order {
        self.drums_lock().set.order
    }

    /// Moves the track at position `from` to position `to`.
    pub fn drum_move_track(&self, from: usize, to: usize) {
        let mut d = self.drums_lock();
        let before = d.set.order;
        move_track(&mut d.set.order, from, to);
        if d.set.order != before {
            self.save_drum_set(&d);
        }
    }

    /// What each instrument's sound is called, where it is known.
    pub fn drum_labels(&self) -> Labels {
        self.drums_lock().kit.labels.clone()
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

    /// Puts sounds (instrument, audio or `None` to remove it, name) into
    /// the user's kit.
    fn set_drum_samples(&self, changes: Vec<(usize, Option<TrackAudio>, Option<String>)>) -> Result<(), String> {
        let kit = self.own_drum_kit()?;
        let drums = self.paths.drums();
        for (inst, audio, label) in &changes {
            match audio {
                Some(a) => kits::set_sample(&drums, &kit, *inst, a, label.as_deref())?,
                None => kits::remove_sample(&drums, &kit, *inst)?,
            }
        }
        let mut d = self.drums_lock();
        for (inst, audio, label) in changes {
            d.kit.labels[inst] = label.filter(|_| audio.is_some());
            d.kit.samples[inst] = audio.map(Arc::new);
        }
        let samples = d.kit.samples.clone();
        drop(d);
        self.send(Command::SetDrumKit(Some(Arc::new(DrumKit { samples }))));
        self.notify(UiEvent::DrumsChanged);
        Ok(())
    }

    /// Loads an audio file as instrument `inst`'s sound (in the background).
    pub fn drum_load_file(self: &Arc<Self>, inst: usize, path: &Path) {
        self.drum_load_files(Some(inst), &[path.to_owned()]);
    }

    /// Loads audio files as the instruments' sounds (in the background).
    /// `into`: the first file goes to that instrument, the others to the
    /// tracks after it; `None`: each to the instrument its name suggests
    /// (`kick.wav`, `open hat.wav` …), the rest to the tracks left.
    pub fn drum_load_files(self: &Arc<Self>, into: Option<usize>, paths: &[PathBuf]) {
        if into.is_some_and(|i| i >= INSTRUMENTS) {
            return;
        }
        let (assigned, left) = kits::assign(paths, &self.drum_order(), into);
        let named = assigned
            .into_iter()
            .map(|(inst, path)| {
                let label = path.file_stem().map(|s| s.to_string_lossy().into_owned());
                (inst, path, label)
            })
            .collect();
        self.load_drum_samples(named, left);
    }

    /// Decodes and loads sounds (instrument, file, name) in the background;
    /// `left`: files there was no track for.
    fn load_drum_samples(self: &Arc<Self>, files: Vec<(usize, PathBuf, Option<String>)>, left: usize) {
        if files.is_empty() {
            return;
        }
        let app = self.clone();
        std::thread::Builder::new()
            .name("drum-sample".into())
            .spawn(move || {
                let several = files.len() > 1;
                let mut changes = Vec::new();
                let mut problems = Vec::new();
                for (inst, path, label) in files {
                    match kits::decode(&path) {
                        Ok(a) => changes.push((inst, Some(a), label)),
                        Err(e) => problems.push(format!("Cannot load {}: {e}", path.display())),
                    }
                }
                let loaded: Vec<String> = changes
                    .iter()
                    .map(|(i, _, l)| format!("{} → {}", l.as_deref().unwrap_or("?"), NAMES[*i]))
                    .collect();
                if !changes.is_empty()
                    && let Err(e) = app.set_drum_samples(changes)
                {
                    problems.push(e);
                } else if several && !loaded.is_empty() {
                    problems.insert(0, format!("Loaded {}", loaded.join(", ")));
                }
                if left > 0 {
                    problems.push(format!("{left} more file(s) left out: there are {INSTRUMENTS} tracks"));
                }
                if !problems.is_empty() {
                    app.notify(UiEvent::Status(problems.join(". ")));
                }
            })
            .expect("spawn drum sample loader");
    }

    /// Loads a library track as instrument `inst`'s sound (its first
    /// seconds; for one-shots in the collection).
    pub fn drum_load_track(self: &Arc<Self>, inst: usize, id: TrackId) {
        if inst >= INSTRUMENTS {
            return;
        }
        let Ok(Some(row)) = self.library.lock().expect("library lock").track(id) else { return };
        let app = self.clone();
        std::thread::Builder::new()
            .name("drum-sample".into())
            .spawn(move || match app.streamed_file(&row, None, None) {
                Ok(p) => app.load_drum_samples(vec![(inst, p, Some(row.title.clone()))], 0),
                Err(e) => app.notify(UiEvent::Status(format!("Cannot stream {}: {e}", row.title))),
            })
            .expect("spawn drum sample loader");
    }

    /// Removes instrument `inst`'s sound from the user's kit.
    pub fn drum_clear_sample(&self, inst: usize) {
        if inst < INSTRUMENTS
            && let Err(e) = self.set_drum_samples(vec![(inst, None, None)])
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
        set.order = [7, 6, 5, 4, 3, 2, 1, 0];
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
        assert_eq!(set.order, DEFAULT_ORDER);
    }
}
