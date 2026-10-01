//! Remix decks: samples triggered in time and synced to a track deck.

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use rille_core::{BeatGrid, BeatMap, Control, ControlEvent, ControlTarget, ControlValue, GridSource};
use rille_engine::{Command, Engine, EngineHandle, HOTCUES, LoadedTrack, RemixSample, TrackAudio, create};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const SR: u32 = 48_000;
const BLOCK: usize = 256;

/// A 5 ms pulse on every beat from `first_beat` (long enough to come
/// through time-stretching at full level).
fn clicks(bpm: f64, first_beat: f64, secs: f64) -> Vec<[f32; 2]> {
    let mut frames = vec![[0.0f32; 2]; (secs * f64::from(SR)) as usize];
    let mut t = first_beat;
    while t < secs - 0.01 {
        let s = (t * f64::from(SR)).round() as usize;
        for f in frames.iter_mut().skip(s).take(240) {
            *f = [0.9, 0.9];
        }
        t += 60.0 / bpm;
    }
    frames
}

fn ctl(h: &EngineHandle, deck: u8, c: Control, v: ControlValue) {
    let target = ControlTarget::deck(deck, c);
    assert!(h.send(Command::Control(ControlEvent { target, value: v })).is_ok());
}

fn press(h: &EngineHandle, deck: u8, c: Control) {
    ctl(h, deck, c, ControlValue::Press(true));
    ctl(h, deck, c, ControlValue::Press(false));
}

fn run(e: &mut Engine, secs: f64) -> Vec<[f32; 2]> {
    let blocks = (secs * f64::from(SR) / BLOCK as f64).ceil() as usize;
    let mut out = Vec::with_capacity(blocks * BLOCK);
    for _ in 0..blocks {
        let mut copy = [[0.0f32; 2]; BLOCK];
        assert_no_alloc(|| {
            let (m, _) = e.render(BLOCK);
            copy[..m.len()].copy_from_slice(m);
        });
        out.extend_from_slice(&copy);
    }
    out
}

fn onsets(out: &[[f32; 2]]) -> Vec<usize> {
    let mut v = Vec::new();
    let mut quiet = 0usize;
    for (i, f) in out.iter().enumerate() {
        if f[0].abs() > 0.3 {
            if quiet > 200 {
                v.push(i);
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    v
}

/// Deck A plays a 128 BPM click track with its fader closed; deck C is a
/// 120 BPM remix deck with a one-bar click loop (also 120 BPM) in cell 1.
fn setup(keylock: bool) -> (EngineHandle, Engine) {
    let (h, e) = create(SR, BLOCK);
    let grid = Arc::new(BeatGrid::new(BeatMap::constant(0.1, 128.0).unwrap(), GridSource::Manual));
    let audio = Arc::new(TrackAudio { sample_rate: SR, frames: clicks(128.0, 0.1, 60.0) });
    let track =
        LoadedTrack { id: 1, audio, grid: Some(grid), main_cue_secs: 0.0, hotcues: [None; HOTCUES], auto_gain_db: 0.0 };
    assert!(h.send(Command::Load { deck: 0, track }).is_ok());
    ctl(&h, 0, Control::Volume, ControlValue::Absolute(0.0));
    if !keylock {
        press(&h, 0, Control::Keylock);
    }
    assert!(h.send(Command::SetRemix { deck: 2, remix: Some(h.new_remix_deck(2, 7, 120.0)) }).is_ok());
    let sample = RemixSample {
        audio: Arc::new(TrackAudio { sample_rate: SR, frames: clicks(120.0, 0.0, 2.0) }),
        bpm: 120.0,
        looped: true,
        color: 3,
    };
    assert!(h.send(Command::SetRemixCell { deck: 2, cell: 0, sample: Some(sample) }).is_ok());
    if !keylock {
        press(&h, 2, Control::Keylock);
    }
    (h, e)
}

/// Worst distance (frames) from each click in `out` (starting at output
/// frame `start`) to deck A's nearest beat, the first at frame `first`.
fn worst_offset(out: &[[f32; 2]], start: usize, first: f64) -> (f64, usize) {
    let beat = 60.0 / 128.0 * f64::from(SR);
    let first = first - start as f64;
    let hits: Vec<usize> = onsets(out).into_iter().filter(|&i| i >= SR as usize).collect();
    let worst = hits
        .iter()
        .map(|&i| {
            let k = ((i as f64 - first) / beat).round();
            (i as f64 - (first + k * beat)).abs()
        })
        .fold(0.0, f64::max);
    (worst, hits.len())
}

fn synced_loop_follows_the_track(keylock: bool) -> f64 {
    let (h, mut e) = setup(keylock);
    // Where deck A's clicks come out (the channel EQ delays a click's onset
    // a little, the same for every deck).
    ctl(&h, 0, Control::Volume, ControlValue::Absolute(1.0));
    press(&h, 0, Control::Play);
    let lead = run(&mut e, 1.3);
    let first = onsets(&lead)[0] as f64;
    ctl(&h, 0, Control::Volume, ControlValue::Absolute(0.0));
    let gap = run(&mut e, 0.05);
    press(&h, 2, Control::Sync);
    // Stopped remix deck: the first trigger starts it.
    press(&h, 2, Control::RemixCell(1));
    let start = lead.len() + gap.len();
    let out = run(&mut e, 8.0);
    let snap = h.snapshot();
    assert!(snap.decks[2].playing && snap.decks[2].remix);
    assert_eq!(snap.remix[2].slots[0].cell, Some(0));
    assert!((snap.decks[2].bpm - 128.0).abs() < 0.01, "follows the clock: {}", snap.decks[2].bpm);
    let (worst, hits) = worst_offset(&out, start, first);
    assert!(hits >= 12, "the loop keeps clicking: {hits}");
    worst
}

#[test]
fn synced_loop_follows_the_track_varispeed() {
    let worst = synced_loop_follows_the_track(false);
    assert!(worst <= 2.0, "clicks within 2 frames of deck A's beats: {worst}");
}

#[test]
fn synced_loop_follows_the_track_keylock() {
    let worst = synced_loop_follows_the_track(true);
    assert!(worst <= 48.0, "clicks within 1 ms of deck A's beats: {worst}");
}

#[test]
fn quantized_trigger_waits_for_the_bar_and_stop_silences() {
    let (h, mut e) = setup(false);
    // No track playing: the remix deck runs on its own at 120 BPM.
    assert!(h.send(Command::Unload { deck: 0 }).is_ok());
    press(&h, 2, Control::Play);
    run(&mut e, 0.75); // 1.5 beats in
    press(&h, 2, Control::RemixPad(1)); // pad 1 = cell 1 on page 1
    run(&mut e, 0.01);
    assert_eq!(h.snapshot().remix[2].slots[0].queued, Some(0), "waits for bar 2");
    let out = run(&mut e, 3.0);
    let hits = onsets(&out);
    // Bar 2 starts at beat 4 = 2.0 s; we are at about 0.76 s.
    let first = hits[0] as f64 / f64::from(SR) + 0.76;
    assert!((first - 2.0).abs() < 0.01, "first click on the bar: {first}");
    press(&h, 2, Control::RemixStop(1));
    let out = run(&mut e, 1.0);
    assert!(onsets(&out).is_empty(), "stopped");
    assert_eq!(h.snapshot().remix[2].slots[0].cell, None);
}

#[test]
fn switching_back_to_a_track_deck() {
    let (h, mut e) = setup(false);
    press(&h, 2, Control::RemixCell(1));
    run(&mut e, 0.5);
    assert!(h.snapshot().decks[2].playing);
    assert!(h.send(Command::SetRemix { deck: 2, remix: None }).is_ok());
    run(&mut e, 0.1);
    let snap = h.snapshot();
    assert!(!snap.decks[2].loaded && !snap.decks[2].remix && !snap.remix[2].active);
    h.poll(|_| {});
}
