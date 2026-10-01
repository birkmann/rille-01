//! Per-deck state for QML (track info, transport, channel strip).

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
        #[qproperty(i32, deck)]
        #[qproperty(bool, loaded)]
        #[qproperty(bool, loading)]
        /// What loading does now: "Streaming from Beatport", "Preparing
        /// the track"…
        #[qproperty(QString, loading_text, cxx_name = "loadingText")]
        /// Download of a streamed track 0..1; −1 when not downloading.
        #[qproperty(f64, download_progress, cxx_name = "downloadProgress")]
        /// "42 % · 3.4 MB/s · 9 s left".
        #[qproperty(QString, download_text, cxx_name = "downloadText")]
        #[qproperty(bool, analyzing)]
        #[qproperty(QString, title)]
        #[qproperty(QString, artist)]
        #[qproperty(QString, info)]
        #[qproperty(QString, cover)]
        #[qproperty(QString, key_text, cxx_name = "keyText")]
        #[qproperty(QString, key_color, cxx_name = "keyColor")]
        #[qproperty(f64, bpm)]
        #[qproperty(f64, track_bpm, cxx_name = "trackBpm")]
        #[qproperty(f64, tempo_percent, cxx_name = "tempoPercent")]
        #[qproperty(f64, tempo_fader, cxx_name = "tempoFader")]
        #[qproperty(f64, position)]
        #[qproperty(f64, duration)]
        #[qproperty(bool, playing)]
        #[qproperty(bool, cue_held, cxx_name = "cueHeld")]
        #[qproperty(bool, sync)]
        #[qproperty(bool, master)]
        #[qproperty(bool, keylock)]
        #[qproperty(bool, flux)]
        #[qproperty(bool, reverse)]
        /// Metronome on the grid (TICK).
        #[qproperty(bool, tick)]
        /// Beat within the bar at the play position, 0..3 (−1 without grid).
        #[qproperty(i32, beat_in_bar, cxx_name = "beatInBar")]
        #[qproperty(bool, loop_active, cxx_name = "loopActive")]
        #[qproperty(bool, loop_set, cxx_name = "loopSet")]
        #[qproperty(f64, loop_start, cxx_name = "loopStart")]
        #[qproperty(f64, loop_end, cxx_name = "loopEnd")]
        #[qproperty(QString, loop_size_text, cxx_name = "loopSizeText")]
        /// Index of the loop size in `loopSizes`.
        #[qproperty(i32, loop_size_index, cxx_name = "loopSizeIndex")]
        #[qproperty(f64, main_cue, cxx_name = "mainCue")]
        #[qproperty(QString, hotcues_json, cxx_name = "hotcuesJson")]
        #[qproperty(f64, phase_error, cxx_name = "phaseError")]
        #[qproperty(f64, beat)]
        #[qproperty(f64, grid_confidence, cxx_name = "gridConfidence")]
        #[qproperty(QString, grid_text, cxx_name = "gridText")]
        #[qproperty(bool, grid_locked, cxx_name = "gridLocked")]
        #[qproperty(bool, has_grid, cxx_name = "hasGrid")]
        #[qproperty(bool, end_warning, cxx_name = "endWarning")]
        #[qproperty(f64, key_shift, cxx_name = "keyShift")]
        #[qproperty(f64, gain)]
        #[qproperty(f64, eq_hi, cxx_name = "eqHi")]
        #[qproperty(f64, eq_mid, cxx_name = "eqMid")]
        #[qproperty(f64, eq_lo, cxx_name = "eqLo")]
        #[qproperty(f64, filter)]
        #[qproperty(f64, volume)]
        #[qproperty(bool, pfl)]
        #[qproperty(bool, fx1)]
        #[qproperty(bool, fx2)]
        #[qproperty(f64, meter_l, cxx_name = "meterL")]
        #[qproperty(f64, meter_r, cxx_name = "meterR")]
        #[qproperty(QString, error)]
        /// A remix deck (the remix properties below are set).
        #[qproperty(bool, remix)]
        /// Page of the pad grid, 0..3.
        #[qproperty(i32, remix_page, cxx_name = "remixPage")]
        #[qproperty(bool, remix_quantize, cxx_name = "remixQuantize")]
        #[qproperty(i32, remix_quant_index, cxx_name = "remixQuantIndex")]
        /// Letter of the deck CAPTURE takes loops from.
        #[qproperty(QString, remix_source, cxx_name = "remixSource")]
        /// 64 cells (`slot * 16 + row`): null or
        /// `{"name","color","loop","bpm"}`. Changes only when cells change.
        #[qproperty(QString, remix_cells_json, cxx_name = "remixCellsJson")]
        /// 4 slots: `{"cell","queued" (-1 = none),"progress","volume",
        /// "filter","muted","meter"}`.
        #[qproperty(QString, remix_slots_json, cxx_name = "remixSlotsJson")]
        type DeckController = super::DeckControllerRust;

        /// Pull the latest state; call once per frame.
        #[qinvokable]
        fn refresh(self: Pin<&mut DeckController>);

        /// Control target string for this deck, e.g. `target("play")` → "deck.A.play".
        #[qinvokable]
        fn target(self: &DeckController, name: &QString) -> QString;

        /// Loop size labels, smallest first ("1/32" … "32").
        #[qinvokable]
        #[cxx_name = "loopSizeLabel"]
        fn loop_size_label(self: &DeckController, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "loopSizeCount"]
        fn loop_size_count(self: &DeckController) -> i32;

        /// Remix quantize size labels ("1/4" … "8").
        #[qinvokable]
        #[cxx_name = "remixQuantLabel"]
        fn remix_quant_label(self: &DeckController, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "remixQuantCount"]
        fn remix_quant_count(self: &DeckController) -> i32;

        /// Edits a remix cell: "delete", "type" (loop/one-shot), "capture"
        /// (from the capture source), "color" (`arg` = colour index).
        #[qinvokable]
        #[cxx_name = "remixCell"]
        fn remix_cell(self: &DeckController, cell: i32, action: &QString, arg: i32);

        /// Loads a library track (`trackId`) into a remix cell.
        #[qinvokable]
        #[cxx_name = "remixLoadTrack"]
        fn remix_load_track(self: &DeckController, cell: i32, track_id: i64);

        /// Loads a file (path token, see AppController) into a remix cell.
        #[qinvokable]
        #[cxx_name = "remixLoadPath"]
        fn remix_load_path(self: &DeckController, cell: i32, token: i64);

        #[qinvokable]
        #[cxx_name = "remixLoadUrl"]
        fn remix_load_url(self: &DeckController, cell: i32, url: &QUrl);

        #[qinvokable]
        #[cxx_name = "renameRemixSet"]
        fn rename_remix_set(self: &DeckController, name: &QString);

        /// Cell colours as "#rrggbb", by colour index.
        #[qinvokable]
        #[cxx_name = "remixColor"]
        fn remix_color(self: &DeckController, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "remixColorCount"]
        fn remix_color_count(self: &DeckController) -> i32;
    }
}

use core::pin::Pin;

use cxx_qt::CxxQtType;

use cxx_qt_lib::{QString, QUrl};
use rille_core::remix::{CELLS, COLORS};
use rille_core::{BeatClock, GridFlags};
use rille_engine::{LOOP_SIZES, REMIX_QUANT_SIZES};

use crate::app_controller::{json_str, url_path};
use crate::global::{app, playhead, token_path};

#[derive(Default)]
pub struct DeckControllerRust {
    deck: i32,
    loaded: bool,
    loading: bool,
    loading_text: QString,
    download_progress: f64,
    download_text: QString,
    analyzing: bool,
    title: QString,
    artist: QString,
    info: QString,
    cover: QString,
    key_text: QString,
    key_color: QString,
    bpm: f64,
    track_bpm: f64,
    tempo_percent: f64,
    tempo_fader: f64,
    position: f64,
    duration: f64,
    playing: bool,
    tick: bool,
    beat_in_bar: i32,
    loop_size_index: i32,
    cue_held: bool,
    sync: bool,
    master: bool,
    keylock: bool,
    flux: bool,
    reverse: bool,
    loop_active: bool,
    loop_set: bool,
    loop_start: f64,
    loop_end: f64,
    loop_size_text: QString,
    main_cue: f64,
    hotcues_json: QString,
    phase_error: f64,
    beat: f64,
    grid_confidence: f64,
    grid_text: QString,
    grid_locked: bool,
    has_grid: bool,
    end_warning: bool,
    key_shift: f64,
    gain: f64,
    eq_hi: f64,
    eq_mid: f64,
    eq_lo: f64,
    filter: f64,
    volume: f64,
    pfl: bool,
    fx1: bool,
    fx2: bool,
    meter_l: f64,
    meter_r: f64,
    error: QString,
    remix: bool,
    remix_page: i32,
    remix_quantize: bool,
    remix_quant_index: i32,
    remix_source: QString,
    remix_cells_json: QString,
    remix_slots_json: QString,
    revision: u64,
}

fn color_hex(index: u8) -> String {
    format!("#{:06x}", COLORS[usize::from(index).min(COLORS.len() - 1)])
}

/// The cells of a remix set for QML (see `remixCellsJson`).
fn cells_json(set: &rille_app::RemixSet) -> String {
    let cells: Vec<String> = set
        .cells
        .iter()
        .map(|c| match c {
            Some(c) => format!(
                r#"{{"name":{},"color":"{}","loop":{},"bpm":{:.2},"ready":{}}}"#,
                json_str(&c.name),
                color_hex(c.color),
                c.looped,
                c.bpm,
                c.audio.is_some()
            ),
            None => "null".into(),
        })
        .collect();
    format!("[{}]", cells.join(","))
}

fn size_text(beats: f64) -> String {
    if beats >= 1.0 { format!("{beats}") } else { format!("1/{}", (1.0 / beats).round()) }
}

/// Hue → "#rrggbb" for the key colour wheel.
fn hue_color(hue: f32) -> String {
    let h = hue.rem_euclid(360.0) / 60.0;
    let (s, v) = (0.62f32, 0.95f32);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |f: f32| ((f + m) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(r), q(g), q(b))
}

pub fn key_color(key: rille_core::Key) -> String {
    hue_color(key.hue_degrees())
}

impl qobject::DeckController {
    fn refresh(mut self: Pin<&mut Self>) {
        let Some(app) = app() else { return };
        let d = (*self.deck()).clamp(0, 3) as usize;
        let snap = app.snapshot();
        let s = snap.decks[d];
        let ch = snap.channels[d];

        // Track info only when it changed.
        let info = app.deck(d as u8);
        if info.revision != self.rust().revision {
            self.as_mut().rust_mut().revision = info.revision;
            let settings = app.settings();
            self.as_mut().set_title(QString::from(info.title.as_str()));
            self.as_mut().set_artist(QString::from(info.artist.as_str()));
            let extra = [info.remixer.as_str(), info.label.as_str()]
                .iter()
                .filter(|t| !t.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" · ");
            self.as_mut().set_info(QString::from(extra));
            self.as_mut().set_cover(QString::from(
                info.cover.as_ref().map_or(String::new(), |p| format!("file://{}", p.display())),
            ));
            self.as_mut().set_key_text(QString::from(info.key.map_or(String::new(), |k| settings.key_text(k))));
            self.as_mut().set_key_color(QString::from(info.key.map_or("#8a9099".into(), key_color)));
            self.as_mut().set_loading(info.loading);
            let (text, progress, detail) = match info.download {
                Some(d) if d.total_bytes == 0 => ("Connecting to Beatport…", 0.0, String::new()),
                Some(d) => (
                    "Streaming from Beatport",
                    f64::from(d.fraction),
                    format!("{} % · {}", (d.fraction * 100.0).floor() as i32, d.describe()),
                ),
                None => ("Preparing the track…", -1.0, String::new()),
            };
            self.as_mut().set_loading_text(QString::from(text));
            self.as_mut().set_download_progress(progress);
            self.as_mut().set_download_text(QString::from(detail));
            self.as_mut().set_analyzing(info.analyzing);
            self.as_mut().set_error(QString::from(info.error.clone().unwrap_or_default()));
            self.as_mut().set_remix_cells_json(QString::from(info.remix.as_deref().map_or("[]".into(), cells_json)));
            match info.grid.as_deref() {
                Some(g) => {
                    let mut notes = Vec::new();
                    let f = g.flags;
                    if f.contains(GridFlags::VARIABLE_TEMPO) {
                        notes.push("variable tempo");
                    }
                    if f.contains(GridFlags::TEMPO_CHANGE) {
                        notes.push("tempo change");
                    }
                    if f.contains(GridFlags::OCTAVE_AMBIGUOUS) {
                        notes.push("check tempo");
                    }
                    if f.contains(GridFlags::DOWNBEAT_UNCERTAIN) {
                        notes.push("check bar start");
                    }
                    if f.contains(GridFlags::NO_RHYTHM) {
                        notes.push("no clear beat");
                    }
                    self.as_mut().set_has_grid(true);
                    self.as_mut().set_grid_confidence(f64::from(g.confidence));
                    self.as_mut().set_grid_text(QString::from(notes.join(", ")));
                    self.as_mut().set_grid_locked(g.locked);
                }
                None => {
                    self.as_mut().set_has_grid(false);
                    self.as_mut().set_grid_confidence(0.0);
                    self.as_mut().set_grid_text(QString::from(if info.analyzing { "analyzing…" } else { "" }));
                    self.as_mut().set_grid_locked(false);
                }
            }
        }

        self.as_mut().set_loaded(s.loaded);
        self.as_mut().set_bpm((s.bpm * 100.0).round() / 100.0);
        self.as_mut().set_track_bpm((s.track_bpm * 100.0).round() / 100.0);
        self.as_mut().set_tempo_percent(((s.rate - 1.0) * 1000.0).round() / 10.0);
        self.as_mut().set_tempo_fader(f64::from(s.tempo_fader));
        self.as_mut().set_position(playhead(d, &snap));
        self.as_mut().set_duration(s.duration_secs);
        self.as_mut().set_playing(s.playing);
        self.as_mut().set_cue_held(s.cue_held);
        self.as_mut().set_sync(s.sync);
        self.as_mut().set_master(s.master);
        self.as_mut().set_keylock(s.keylock);
        self.as_mut().set_flux(s.flux);
        self.as_mut().set_reverse(s.reverse);
        self.as_mut().set_tick(s.tick);
        self.as_mut().set_loop_active(s.loop_active);
        self.as_mut().set_loop_set(s.loop_set);
        self.as_mut().set_loop_start(s.loop_start_secs);
        self.as_mut().set_loop_end(s.loop_end_secs);
        self.as_mut().set_loop_size_text(QString::from(size_text(LOOP_SIZES[s.loop_size_idx])));
        self.as_mut().set_loop_size_index(s.loop_size_idx as i32);
        self.as_mut().set_main_cue(s.main_cue_secs);
        let cues: Vec<String> = s
            .hotcues
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                h.map(|h| {
                    let color = rille_core::track::HOTCUE_COLORS[i];
                    let loop_ = h.kind == rille_core::CueKind::Loop;
                    format!(
                        r##"{{"slot":{i},"secs":{:.6},"color":"#{color:06x}","loop":{loop_},"len":{:.6}}}"##,
                        h.secs, h.len_secs
                    )
                })
            })
            .collect();
        self.as_mut().set_hotcues_json(QString::from(format!("[{}]", cues.join(","))));
        self.as_mut().set_phase_error(s.phase_error);
        self.as_mut().set_beat(s.beat);
        self.as_mut().set_end_warning(s.end_warning);
        self.as_mut().set_key_shift(f64::from(s.key_shift));
        self.as_mut().set_gain(f64::from(ch.gain));
        self.as_mut().set_eq_hi(f64::from(ch.eq[2]));
        self.as_mut().set_eq_mid(f64::from(ch.eq[1]));
        self.as_mut().set_eq_lo(f64::from(ch.eq[0]));
        self.as_mut().set_filter(f64::from(ch.filter));
        self.as_mut().set_volume(f64::from(ch.volume));
        self.as_mut().set_pfl(ch.pfl);
        self.as_mut().set_fx1(ch.fx_assign[0]);
        self.as_mut().set_fx2(ch.fx_assign[1]);
        self.as_mut().set_meter_l(f64::from(ch.meter[0]));
        self.as_mut().set_meter_r(f64::from(ch.meter[1]));
        let in_bar = info.grid.as_deref().filter(|_| s.loaded).map_or(-1, |g| {
            let b = g.beat_at(playhead(d, &snap)).floor() as i64;
            (0..4).find(|k| g.is_downbeat(b - k)).unwrap_or(0) as i32
        });
        self.as_mut().set_beat_in_bar(in_bar);

        let r = &snap.remix[d];
        self.as_mut().set_remix(s.remix);
        if s.remix {
            self.as_mut().set_remix_page(i32::from(r.page));
            self.as_mut().set_remix_quantize(r.quantize);
            self.as_mut().set_remix_quant_index(i32::from(r.quant_idx));
            self.as_mut().set_remix_source(QString::from(char::from(b'A' + r.capture_source).to_string()));
            let opt = |c: Option<u8>| c.map_or(-1, i32::from);
            let slots: Vec<String> = r
                .slots
                .iter()
                .map(|sl| {
                    format!(
                        r#"{{"cell":{},"queued":{},"progress":{:.4},"volume":{:.3},"filter":{:.3},"muted":{},"meter":{:.4}}}"#,
                        opt(sl.cell),
                        opt(sl.queued),
                        sl.progress,
                        sl.volume,
                        sl.filter,
                        sl.muted,
                        sl.meter[0].max(sl.meter[1])
                    )
                })
                .collect();
            self.as_mut().set_remix_slots_json(QString::from(format!("[{}]", slots.join(","))));
        }
    }

    fn remix_quant_label(&self, index: i32) -> QString {
        QString::from(
            usize::try_from(index).ok().and_then(|i| REMIX_QUANT_SIZES.get(i)).map_or(String::new(), |b| size_text(*b)),
        )
    }

    fn remix_quant_count(&self) -> i32 {
        REMIX_QUANT_SIZES.len() as i32
    }

    fn deck_index(&self) -> u8 {
        (*self.deck()).clamp(0, 3) as u8
    }

    fn remix_cell(&self, cell: i32, action: &QString, arg: i32) {
        let (Some(app), Ok(cell)) = (app(), usize::try_from(cell)) else { return };
        if cell >= CELLS {
            return;
        }
        let deck = self.deck_index();
        match action.to_string().as_str() {
            "delete" => app.delete_remix_cell(deck, cell),
            "type" => app.toggle_remix_cell_type(deck, cell),
            "capture" => app.capture_remix_cell(deck, cell),
            "color" => app.set_remix_cell_color(deck, cell, arg.clamp(0, 255) as u8),
            _ => {}
        }
    }

    fn remix_load_track(&self, cell: i32, track_id: i64) {
        if let Some(app) = app() {
            app.load_remix_cell(self.deck_index(), usize::try_from(cell).ok(), track_id);
        }
    }

    fn remix_load_path(&self, cell: i32, token: i64) {
        if let (Some(app), Some(p)) = (app(), token_path(token)) {
            app.load_remix_file(self.deck_index(), usize::try_from(cell).ok(), &p);
        }
    }

    fn remix_load_url(&self, cell: i32, url: &QUrl) {
        if let Some(app) = app() {
            app.load_remix_file(self.deck_index(), usize::try_from(cell).ok(), &url_path(url));
        }
    }

    fn rename_remix_set(&self, name: &QString) {
        if let Some(app) = app() {
            app.rename_remix_set(self.deck_index(), &name.to_string());
        }
    }

    fn remix_color(&self, index: i32) -> QString {
        QString::from(color_hex(index.clamp(0, 255) as u8))
    }

    fn remix_color_count(&self) -> i32 {
        COLORS.len() as i32
    }

    fn loop_size_label(&self, index: i32) -> QString {
        QString::from(
            usize::try_from(index).ok().and_then(|i| LOOP_SIZES.get(i)).map_or(String::new(), |b| size_text(*b)),
        )
    }

    fn loop_size_count(&self) -> i32 {
        LOOP_SIZES.len() as i32
    }

    fn target(&self, name: &QString) -> QString {
        let letter = char::from(b'A' + (*self.deck()).clamp(0, 3) as u8);
        QString::from(format!("deck.{letter}.{name}"))
    }
}
