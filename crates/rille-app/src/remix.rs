//! Remix decks on the app side: which decks are remix decks, what their
//! cells hold (a region of an audio file, its name, tempo and colour), and
//! where samples come from: a file or library track loaded into a cell, or a
//! loop captured from a track deck. Each deck's set is saved under
//! `<data>/remix/deck-<letter>.toml` after every change and comes back on
//! the next start.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rille_core::remix::{CELLS, PAGE_ROWS, PAGES, ROWS, SLOT_COLORS, SLOTS, pad_cell};
use rille_core::{BeatClock, BeatGrid};
use rille_engine::{Command, LOOP_SIZES, MAX_DECKS, RemixSample, TrackAudio, loop_start_beat, remix_grid};
use rille_library::TrackId;
use serde::{Deserialize, Serialize};

use crate::{App, DeckInfo, UiEvent, analysis_config};

/// A sample in a remix cell: a region of an audio file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemixCell {
    pub name: String,
    pub path: PathBuf,
    pub start_secs: f64,
    pub len_secs: f64,
    /// The region's tempo; it always plays at the deck's tempo.
    pub bpm: f64,
    pub looped: bool,
    /// Index into `rille_core::remix::COLORS`.
    pub color: u8,
    /// The decoded region, once loaded.
    #[serde(skip)]
    pub audio: Option<Arc<TrackAudio>>,
}

/// Equal descriptions (the decoded audio is not compared).
impl PartialEq for RemixCell {
    fn eq(&self, o: &Self) -> bool {
        (&self.name, &self.path, self.start_secs, self.len_secs, self.bpm, self.looped, self.color)
            == (&o.name, &o.path, o.start_secs, o.len_secs, o.bpm, o.looped, o.color)
    }
}

impl RemixCell {
    fn sample(&self) -> Option<RemixSample> {
        Some(RemixSample { audio: self.audio.clone()?, bpm: self.bpm, looped: self.looped, color: self.color })
    }
}

/// The cells of one remix deck.
#[derive(Clone, Debug, PartialEq)]
pub struct RemixSet {
    pub name: String,
    /// The deck's own tempo (when not synced).
    pub bpm: f64,
    /// [`CELLS`] cells, `slot * 16 + row`.
    pub cells: Vec<Option<RemixCell>>,
}

#[derive(Serialize, Deserialize)]
struct SetFile {
    name: String,
    bpm: f64,
    #[serde(default, rename = "cell")]
    cells: Vec<CellEntry>,
}

#[derive(Serialize, Deserialize)]
struct CellEntry {
    /// 1..=64 (`slot * 16 + row + 1`).
    cell: usize,
    #[serde(flatten)]
    sample: RemixCell,
}

impl RemixSet {
    pub fn new(name: impl Into<String>, bpm: f64) -> Self {
        Self { name: name.into(), bpm, cells: vec![None; CELLS] }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(Option::is_none)
    }

    pub fn load(path: &Path) -> Option<Self> {
        let f: SetFile = toml::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let mut set = Self::new(f.name, if f.bpm > 0.0 { f.bpm } else { 120.0 });
        for e in f.cells {
            if let Some(c) = e.cell.checked_sub(1).and_then(|i| set.cells.get_mut(i)) {
                *c = Some(e.sample);
            }
        }
        Some(set)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let cells = self
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some(CellEntry { cell: i + 1, sample: c.clone()? }))
            .collect();
        let text = toml::to_string_pretty(&SetFile { name: self.name.clone(), bpm: self.bpm, cells })
            .map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

fn letter(deck: u8) -> char {
    char::from(b'A' + deck)
}

/// "4 beats", "2 bars", "1/2 beat".
pub fn beats_label(beats: f64) -> String {
    if beats >= 4.0 && (beats / 4.0).fract().abs() < 1e-6 {
        let bars = beats / 4.0;
        format!("{bars} bar{}", if bars == 1.0 { "" } else { "s" })
    } else if beats >= 1.0 {
        format!("{} beat{}", (beats * 100.0).round() / 100.0, if beats == 1.0 { "" } else { "s" })
    } else {
        format!("1/{} beat", (1.0 / beats).round())
    }
}

/// Frames `[start, start + len)` seconds of `frames` as a new sample.
fn slice(frames: &[[f32; 2]], sample_rate: u32, start_secs: f64, len_secs: f64) -> Arc<TrackAudio> {
    let sr = f64::from(sample_rate);
    let a = ((start_secs * sr).round().max(0.0) as usize).min(frames.len());
    let b = (((start_secs + len_secs) * sr).round().max(0.0) as usize).clamp(a, frames.len());
    Arc::new(TrackAudio { sample_rate, frames: frames[a..b].to_vec() })
}

/// Which part of a file a cell plays, and how: `(start, length, bpm, looped)`.
/// With a beatgrid: from the first downbeat; up to 16 bars play as a loop of
/// whole beats, anything longer (a whole track) once. Without one, a short
/// file is taken as a loop of 1, 2, 4, 8 or 16 bars, whichever gives the
/// most likely tempo; otherwise it plays once at `fallback_bpm`.
pub fn sample_region(duration: f64, grid: Option<&BeatGrid>, fallback_bpm: f64) -> (f64, f64, f64, bool) {
    if let Some(g) = grid {
        let mut b = (g.beat_at(0.0) - 1e-3).ceil();
        for _ in 0..8 {
            if g.is_downbeat(b as i64) {
                break;
            }
            b += 1.0;
        }
        let start = g.secs_at(b).max(0.0);
        let beats = g.beat_at(duration) - b;
        if (1.0..=64.5).contains(&beats) {
            let n = if (beats - beats.round()).abs() < 0.1 { beats.round() } else { beats.floor() };
            let len = g.secs_at(b + n) - start;
            return (start, len, n * 60.0 / len, true);
        }
        if beats > 0.0 {
            return (start, duration - start, g.bpm_at(start), false);
        }
    }
    let likely = [4.0, 8.0, 16.0, 32.0, 64.0]
        .into_iter()
        .map(|beats| beats * 60.0 / duration.max(1e-3))
        .filter(|bpm| (80.0..180.0).contains(bpm))
        .min_by(|a, b| (a - 125.0).abs().total_cmp(&(b - 125.0).abs()));
    match likely {
        Some(bpm) => (0.0, duration, bpm, true),
        None => (0.0, duration, fallback_bpm, false),
    }
}

impl App {
    fn remix_file(&self, deck: u8) -> PathBuf {
        self.paths.data.join("remix").join(format!("deck-{}.toml", letter(deck)))
    }

    pub fn is_remix_deck(&self, deck: u8) -> bool {
        self.deck(deck).remix.is_some()
    }

    /// Makes decks remix or track decks as the settings say.
    pub(crate) fn apply_deck_types(self: &Arc<Self>) {
        let want = self.settings().remix_decks.to_uppercase();
        for deck in 0..MAX_DECKS as u8 {
            let wanted = want.contains(letter(deck));
            match (wanted, self.is_remix_deck(deck)) {
                (true, false) => self.make_remix(deck),
                (false, true) => self.make_track(deck),
                _ => {}
            }
        }
        self.refollow_remix_decks();
    }

    fn make_remix(self: &Arc<Self>, deck: u8) {
        let set = RemixSet::load(&self.remix_file(deck))
            .unwrap_or_else(|| RemixSet::new(format!("Remix Deck {}", letter(deck)), 120.0));
        let seq = self.load_seq.fetch_add(1, Ordering::Relaxed);
        {
            let mut info = self.decks[usize::from(deck)].write().expect("deck lock");
            let revision = info.revision + 1;
            *info = DeckInfo {
                engine_id: seq,
                title: set.name.clone(),
                grid: Some(Arc::new(remix_grid(set.bpm))),
                remix: Some(Arc::new(set)),
                revision,
                ..DeckInfo::default()
            };
        }
        self.install_remix(deck);
        self.notify(UiEvent::DeckChanged(deck));
        let app = self.clone();
        std::thread::Builder::new()
            .name(format!("remix-load-{}", letter(deck)))
            .spawn(move || app.decode_cells(deck, seq))
            .expect("spawn remix loader");
    }

    fn make_track(&self, deck: u8) {
        self.send(Command::SetRemix { deck, remix: None });
        let mut info = self.decks[usize::from(deck)].write().expect("deck lock");
        let revision = info.revision + 1;
        *info = DeckInfo { revision, ..DeckInfo::default() };
        drop(info);
        self.notify(UiEvent::DeckChanged(deck));
    }

    /// Sends the remix deck and its decoded cells to the engine (after
    /// switching, or when the audio engine was restarted).
    pub(crate) fn install_remix(&self, deck: u8) {
        let info = self.deck(deck);
        let (Some(set), Some(engine)) = (info.remix, self.engine.get()) else { return };
        let remix = engine.new_remix_deck(deck, info.engine_id, set.bpm);
        self.send(Command::SetRemix { deck, remix: Some(remix) });
        for (cell, c) in set.cells.iter().enumerate() {
            if let Some(sample) = c.as_ref().and_then(RemixCell::sample) {
                self.send(Command::SetRemixCell { deck, cell: cell as u8, sample: Some(sample) });
            }
        }
    }

    /// Decodes the saved cells of a remix deck that was just set up; files
    /// shared by several cells are decoded once.
    fn decode_cells(&self, deck: u8, seq: u64) {
        let Some(set) = self.deck(deck).remix else { return };
        let mut files: HashMap<PathBuf, Option<rille_decode::DecodedAudio>> = HashMap::new();
        for (cell, c) in set.cells.iter().enumerate() {
            let Some(c) = c.as_ref().filter(|c| c.audio.is_none()) else { continue };
            let audio = files
                .entry(c.path.clone())
                .or_insert_with(|| rille_decode::decode_file(&c.path, None, &mut |_| {}).ok());
            let Some(a) = audio else {
                self.notify(UiEvent::Status(format!("Remix deck {}: cannot read {}", letter(deck), c.path.display())));
                continue;
            };
            let sample = slice(&a.frames, a.sample_rate, c.start_secs, c.len_secs);
            let mut c = c.clone();
            c.audio = Some(sample);
            if !self.set_cell(deck, Some(seq), cell, Some(c), false) {
                return;
            }
        }
    }

    /// Puts `c` into `cell` (engine and set) if `deck` is still the remix
    /// deck `seq` (any remix deck for `None`); saves when `save`. The first
    /// sample in an empty deck sets its tempo.
    fn set_cell(&self, deck: u8, seq: Option<u64>, cell: usize, c: Option<RemixCell>, save: bool) -> bool {
        let d = usize::from(deck);
        if d >= MAX_DECKS || cell >= CELLS {
            return false;
        }
        let mut info = self.decks[d].write().expect("deck lock");
        if seq.is_some_and(|s| s != info.engine_id) {
            return false;
        }
        let engine_id = info.engine_id;
        let Some(set) = info.remix.as_mut().map(Arc::make_mut) else { return false };
        let new_tempo = match &c {
            Some(c) if set.is_empty() && c.bpm > 0.0 && (c.bpm - set.bpm).abs() > 1e-6 => Some(c.bpm),
            _ => None,
        };
        let sample = c.as_ref().and_then(RemixCell::sample);
        set.cells[cell] = c;
        if let Some(bpm) = new_tempo {
            set.bpm = bpm;
        }
        let saved = save.then(|| set.clone());
        if let Some(bpm) = new_tempo {
            let grid = Arc::new(remix_grid(bpm));
            info.grid = Some(grid.clone());
            self.send(Command::SetGrid { deck, track_id: engine_id, grid: Some(grid) });
        }
        info.revision += 1;
        drop(info);
        self.send(Command::SetRemixCell { deck, cell: cell as u8, sample });
        if let Some(set) = saved
            && let Err(e) = set.save(&self.remix_file(deck))
        {
            self.notify(UiEvent::Status(format!("Cannot save the remix deck: {e}")));
        }
        self.notify(UiEvent::DeckChanged(deck));
        true
    }

    /// Changes a loaded cell and sends it again.
    fn edit_cell(&self, deck: u8, cell: usize, f: impl FnOnce(&mut RemixCell)) {
        let Some(mut c) = self.deck(deck).remix.and_then(|s| s.cells.get(cell).cloned().flatten()) else { return };
        f(&mut c);
        self.set_cell(deck, None, cell, Some(c), true);
    }

    pub fn delete_remix_cell(&self, deck: u8, cell: usize) {
        if self.deck(deck).remix.is_some_and(|s| s.cells.get(cell).is_some_and(Option::is_some)) {
            self.set_cell(deck, None, cell, None, true);
        }
    }

    /// Switches a cell between loop and one-shot.
    pub fn toggle_remix_cell_type(&self, deck: u8, cell: usize) {
        self.edit_cell(deck, cell, |c| c.looped = !c.looped);
    }

    pub fn set_remix_cell_color(&self, deck: u8, cell: usize, color: u8) {
        self.edit_cell(deck, cell, |c| c.color = color.min(rille_core::remix::COLORS.len() as u8 - 1));
    }

    /// Empties every cell.
    pub fn clear_remix(&self, deck: u8) {
        for cell in 0..CELLS {
            self.delete_remix_cell(deck, cell);
        }
    }

    pub fn rename_remix_set(&self, deck: u8, name: &str) {
        let d = usize::from(deck).min(MAX_DECKS - 1);
        let mut info = self.decks[d].write().expect("deck lock");
        let Some(set) = info.remix.as_mut().map(Arc::make_mut) else { return };
        set.name = name.trim().to_owned();
        let set = set.clone();
        info.title = set.name.clone();
        info.revision += 1;
        drop(info);
        let _ = set.save(&self.remix_file(deck));
        self.notify(UiEvent::DeckChanged(deck));
    }

    /// Cell of pad `pad` (1..=16) on the deck's current page.
    pub fn remix_pad_cell(&self, deck: u8, pad: u8) -> Option<usize> {
        let page = self.snapshot().remix[usize::from(deck).min(MAX_DECKS - 1)].page;
        pad_cell(pad, usize::from(page))
    }

    /// First empty cell, looking at the visible page first.
    fn free_cell(&self, deck: u8) -> Option<usize> {
        let set = self.deck(deck).remix?;
        let page = usize::from(self.snapshot().remix[usize::from(deck).min(MAX_DECKS - 1)].page).min(PAGES - 1);
        let rows = (page * PAGE_ROWS..ROWS).chain(0..page * PAGE_ROWS);
        rows.flat_map(|row| (0..SLOTS).map(move |slot| slot * ROWS + row)).find(|&c| set.cells[c].is_none())
    }

    /// Loads a library track into a cell (decoding and, for whole tracks
    /// without a beatgrid, analysis run in the background).
    pub fn load_remix_cell(self: &Arc<Self>, deck: u8, cell: Option<usize>, id: TrackId) {
        let info = self.deck(deck);
        let Some(set) = info.remix else { return };
        let Some(cell) = cell.or_else(|| self.free_cell(deck)).filter(|&c| c < CELLS) else {
            self.notify(UiEvent::Status(format!("Remix deck {} is full", letter(deck))));
            return;
        };
        let Ok(Some(mut row)) = self.library.lock().expect("library lock").track(id) else { return };
        let (app, seq, fallback_bpm) = (self.clone(), info.engine_id, set.bpm);
        std::thread::Builder::new()
            .name(format!("remix-cell-{}", letter(deck)))
            .spawn(move || {
                // A streamed track out of the cache downloads first.
                match app.streamed_file(&row, None, None) {
                    Ok(p) => row.path = p,
                    Err(e) => {
                        app.notify(UiEvent::Status(format!("Cannot stream {}: {e}", row.title)));
                        return;
                    }
                }
                let audio = match rille_decode::decode_file(&row.path, None, &mut |_| {}) {
                    Ok(a) => a,
                    Err(e) => {
                        app.notify(UiEvent::Status(format!("Cannot decode {}: {e}", row.path.display())));
                        return;
                    }
                };
                let duration = audio.frames.len() as f64 / f64::from(audio.sample_rate.max(1));
                let stored = app.library.lock().expect("library lock").analysis(id).ok().flatten();
                let mut grid = stored.and_then(|a| a.grid);
                if grid.is_none() && duration > 30.0 {
                    // A whole track: find its beats (short loops go by length).
                    let out = rille_analysis::analyze(&audio, &analysis_config(&app.settings()));
                    let mut lib = app.library.lock().expect("library lock");
                    let _ = lib.set_analysis(id, &out.analysis);
                    let _ = lib.set_waveform(id, &out.waveform.to_bytes());
                    grid = out.analysis.grid;
                }
                let (start, len, bpm, looped) = sample_region(duration, grid.as_ref(), fallback_bpm);
                let name = if row.title.is_empty() {
                    row.path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned())
                } else {
                    row.title.clone()
                };
                let c = RemixCell {
                    name,
                    path: row.path.clone(),
                    start_secs: start,
                    len_secs: len,
                    bpm,
                    looped,
                    color: SLOT_COLORS[cell / ROWS],
                    audio: Some(slice(&audio.frames, audio.sample_rate, start, len)),
                };
                app.set_cell(deck, Some(seq), cell, Some(c), true);
            })
            .expect("spawn remix cell loader");
    }

    /// Loads a file (drag and drop, file browser) into a cell; a file outside
    /// the collection becomes a guest track (see [`App::load_file`]).
    pub fn load_remix_file(self: &Arc<Self>, deck: u8, cell: Option<usize>, path: &Path) {
        let id = self.library.lock().expect("library lock").import_file_guest(path);
        match id {
            Ok(id) => {
                self.refresh_track(id);
                self.load_remix_cell(deck, cell, id);
            }
            Err(e) => self.notify(UiEvent::Status(format!("Cannot load {}: {e}", path.display()))),
        }
    }

    /// Captures a loop from the deck's capture source into `cell`: the
    /// source's active loop, or a loop of its loop size from the current
    /// beat.
    pub fn capture_remix_cell(&self, deck: u8, cell: usize) {
        let snap = self.snapshot();
        let rs = &snap.remix[usize::from(deck).min(MAX_DECKS - 1)];
        if !rs.active || cell >= CELLS {
            return;
        }
        // The capture source, or else the master deck, a playing deck, any
        // deck with a track (and say so).
        let usable = |d: u8| {
            let i = self.deck(d);
            d != deck && i.remix.is_none() && i.audio.is_some() && i.grid.is_some() && i.path.is_some()
        };
        let others = || (0..MAX_DECKS as u8).filter(|&d| usable(d));
        let Some(src) = Some(rs.capture_source)
            .filter(|&d| usable(d))
            .or(snap.master_deck.filter(|&d| usable(d)))
            .or_else(|| others().find(|&d| snap.decks[usize::from(d)].playing))
            .or_else(|| others().next())
        else {
            self.notify(UiEvent::Status("Capture: no deck has a track with a beatgrid".into()));
            return;
        };
        if src != rs.capture_source {
            self.notify(UiEvent::Status(format!(
                "Captured from deck {} (deck {} has no track with a beatgrid)",
                letter(src),
                letter(rs.capture_source)
            )));
        }
        let info = self.deck(src);
        let (Some(audio), Some(grid), Some(path)) = (info.audio.clone(), info.grid.clone(), info.path.clone()) else {
            return;
        };
        let ds = &snap.decks[usize::from(src)];
        let (start, end) = if ds.loop_active && ds.loop_set {
            (ds.loop_start_secs, ds.loop_end_secs)
        } else {
            let size = LOOP_SIZES[ds.loop_size_idx.min(LOOP_SIZES.len() - 1)];
            let unit = size.min(1.0);
            let b = loop_start_beat(grid.beat_at(ds.position_secs), unit);
            (grid.secs_at(b), grid.secs_at(b + size))
        };
        if end <= start {
            return;
        }
        let beats = grid.beat_at(end) - grid.beat_at(start);
        let title = if info.title.is_empty() { format!("Deck {}", letter(src)) } else { info.title.clone() };
        let c = RemixCell {
            name: format!("{title} · {}", beats_label(beats)),
            path,
            start_secs: start,
            len_secs: end - start,
            bpm: beats * 60.0 / (end - start),
            looped: true,
            color: SLOT_COLORS[cell / ROWS],
            audio: Some(slice(&audio.frames, audio.sample_rate, start, end - start)),
        };
        self.set_cell(deck, None, cell, Some(c), true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rille_core::{BeatMap, GridSource};

    #[test]
    fn remix_controllers_follow_the_remix_deck() {
        let dir = tempfile::tempdir().unwrap();
        let mappings = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mappings");
        let opts = crate::StartOptions {
            paths: crate::Paths::under(dir.path()),
            audio: false,
            midi: false,
            bundled_mappings: Some(mappings),
        };
        let app = App::start(opts).unwrap();
        let port = "Traktor Kontrol F1 HID (08C2C27B)";
        let pick = |app: &App| app.pick_mapping(&app.mappings.lock().unwrap(), port, &[]).map(|m| m.name);
        assert_eq!(pick(&app).as_deref(), Some("Traktor Kontrol F1 (C)"));
        let mut s = app.settings();
        s.remix_decks = "D".into();
        app.set_settings(s.clone());
        assert_eq!(pick(&app).as_deref(), Some("Traktor Kontrol F1 (D)"));
        // A mapping the user chose stays.
        s.midi_mappings.insert(rille_midi::port_base_name(port).into(), "Traktor Kontrol F1 (A)".into());
        app.set_settings(s);
        assert_eq!(pick(&app).as_deref(), Some("Traktor Kontrol F1 (A)"));
        app.shutdown();
    }

    #[test]
    fn sets_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("remix/deck-C.toml");
        let mut set = RemixSet::new("Live", 124.0);
        set.cells[17] = Some(RemixCell {
            name: "Kick · 1 bar".into(),
            path: "/music/a b.flac".into(),
            start_secs: 12.5,
            len_secs: 1.935,
            bpm: 124.0,
            looped: true,
            color: 3,
            audio: None,
        });
        set.save(&p).unwrap();
        assert_eq!(RemixSet::load(&p), Some(set));
        assert_eq!(RemixSet::load(&dir.path().join("missing.toml")), None);
    }

    #[test]
    fn regions() {
        let grid = BeatGrid::new(BeatMap::constant(0.25, 120.0).unwrap(), GridSource::Manual);
        // A 2-bar loop file with a grid from 0.25 s.
        let (start, len, bpm, looped) = sample_region(4.25, Some(&grid), 100.0);
        assert!((start - 0.25).abs() < 1e-9 && (len - 4.0).abs() < 1e-9 && looped);
        assert!((bpm - 120.0).abs() < 1e-9);
        // A whole track plays once.
        let (_, len, bpm, looped) = sample_region(300.0, Some(&grid), 100.0);
        assert!(!looped && (len - 299.75).abs() < 1e-9 && (bpm - 120.0).abs() < 1e-9);
        // No grid: a 1.875 s file is a 1-bar loop at 128 BPM (or 2 bars at 256: out of range).
        let (_, _, bpm, looped) = sample_region(1.875, None, 100.0);
        assert!(looped && (bpm - 128.0).abs() < 1e-9, "{bpm}");
        let (_, _, bpm, looped) = sample_region(0.3, None, 100.0);
        assert!(!looped && bpm == 100.0);
        assert_eq!(beats_label(8.0), "2 bars");
        assert_eq!(beats_label(4.0), "1 bar");
        assert_eq!(beats_label(2.0), "2 beats");
        assert_eq!(beats_label(0.5), "1/2 beat");
    }
}
