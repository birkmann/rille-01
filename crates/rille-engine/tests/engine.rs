//! Engine behaviour on click tracks with exactly known beats.

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use rille_core::{BeatClock, BeatGrid, BeatMap, Control, ControlEvent, ControlTarget, ControlValue, GridSource};
use rille_engine::{Command, Engine, EngineHandle, HOTCUES, LoadedTrack, TrackAudio, create};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const SR: u32 = 48_000;
const BLOCK: usize = 256;

/// A 1 ms pulse at every beat, silence elsewhere.
fn click_track(bpm: f64, first_beat: f64, secs: f64, sr: u32) -> (Arc<TrackAudio>, Arc<BeatGrid>) {
    let n = (secs * f64::from(sr)) as usize;
    let mut frames = vec![[0.0f32; 2]; n];
    let beat = 60.0 / bpm;
    let mut t = first_beat;
    while t < secs - 0.01 {
        let s = (t * f64::from(sr)).round() as usize;
        for k in 0..(f64::from(sr) * 0.001) as usize {
            if let Some(f) = frames.get_mut(s + k) {
                *f = [0.9, 0.9];
            }
        }
        t += beat;
    }
    let grid = BeatGrid::new(BeatMap::constant(first_beat, bpm).unwrap(), GridSource::Manual);
    (Arc::new(TrackAudio { sample_rate: sr, frames }), Arc::new(grid))
}

fn load(h: &EngineHandle, deck: u8, id: u64, (audio, grid): (Arc<TrackAudio>, Arc<BeatGrid>), cue: f64) {
    let track =
        LoadedTrack { id, audio, grid: Some(grid), main_cue_secs: cue, hotcues: [None; HOTCUES], auto_gain_db: 0.0 };
    assert!(h.send(Command::Load { deck, track }).is_ok());
}

fn ctl(h: &EngineHandle, deck: u8, c: Control, v: ControlValue) {
    let target = ControlTarget::deck(deck, c);
    assert!(h.send(Command::Control(ControlEvent { target, value: v })).is_ok());
}

fn press(h: &EngineHandle, deck: u8, c: Control) {
    ctl(h, deck, c, ControlValue::Press(true));
    ctl(h, deck, c, ControlValue::Press(false));
}

/// Renders `secs` seconds and returns the master output.
fn run(e: &mut Engine, secs: f64) -> Vec<[f32; 2]> {
    let blocks = (secs * f64::from(SR) / BLOCK as f64).ceil() as usize;
    let mut out = Vec::with_capacity(blocks * BLOCK);
    for _ in 0..blocks {
        let block: Vec<[f32; 2]> = {
            let mut copy = [[0.0f32; 2]; BLOCK];
            assert_no_alloc(|| {
                let (m, _) = e.render(BLOCK);
                copy[..m.len()].copy_from_slice(m);
            });
            copy.to_vec()
        };
        out.extend_from_slice(&block);
    }
    out
}

/// Output frames where a click starts (rising above half scale after silence).
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

/// Output latency of the master bus: the limiter's 1 ms look-ahead.
const LIMITER_FRAMES: f64 = 48.0;

/// Plays a click track and returns (worst, mean) offset of the output clicks
/// from where the grid says they should be heard, in ms.
fn timing(keylock: bool, rate_fader: f32) -> (f64, f64) {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(120.0, 0.25, 60.0, 44_100), 0.0);
    if !keylock {
        press(&h, 0, Control::Keylock);
    }
    ctl(&h, 0, Control::Tempo, ControlValue::Absolute(rate_fader));
    press(&h, 0, Control::Play);
    let out = run(&mut e, 20.0);
    let s = h.snapshot().decks[0];
    assert!(s.playing);
    let rate = s.rate;
    let on = onsets(&out);
    assert!(on.len() >= 30, "{} clicks", on.len());
    let offs: Vec<f64> = on
        .iter()
        .skip(1)
        .map(|&f| {
            let t = (f as f64 - LIMITER_FRAMES) / f64::from(SR);
            // Track time heard at output time t (started at 0 at 1.0x `rate`).
            let track_t = t * rate;
            let phase = ((track_t - 0.25) / 0.5).rem_euclid(1.0);
            let d = if phase > 0.5 { phase - 1.0 } else { phase };
            d * 0.5 / rate * 1000.0
        })
        .collect();
    let worst = offs.iter().fold(0.0f64, |m, o| m.max(o.abs()));
    (worst, offs.iter().sum::<f64>() / offs.len() as f64)
}

#[test]
fn plays_at_the_right_speed() {
    for (keylock, fader) in [(false, 0.5), (false, 0.8), (true, 0.5), (true, 0.8), (true, 0.2)] {
        let (worst, mean) = timing(keylock, fader);
        eprintln!("keylock {keylock} fader {fader}: worst {worst:.3} ms, mean {mean:.3} ms");
        assert!(worst < 0.5, "keylock {keylock} fader {fader}: clicks off by up to {worst:.3} ms");
    }
}

/// Deck B (128 bpm, synced) follows deck A (124 bpm). Checked on the audio:
/// B's clicks must land on A's beats.
fn sync_test(keylock: bool, start_b: f64) -> f64 {
    sync_test_bpm(124.0, 128.0, keylock, start_b)
}

/// Follower at `follower_bpm` synced to a master at `master_bpm`; worst
/// distance (ms) of the follower's clicks from the master's beats.
fn sync_test_bpm(master_bpm: f64, follower_bpm: f64, keylock: bool, start_b: f64) -> f64 {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(master_bpm, 0.1, 400.0, 44_100), 0.0);
    load(&h, 1, 2, click_track(follower_bpm, 0.37, 400.0, 48_000), start_b);
    if !keylock {
        press(&h, 1, Control::Keylock);
    }
    press(&h, 0, Control::Play);
    run(&mut e, 2.0);
    press(&h, 1, Control::Sync);
    press(&h, 1, Control::Play);
    // Only deck B audible.
    ctl(&h, 0, Control::Volume, ControlValue::Absolute(0.0));
    run(&mut e, 3.0);
    let rendered = h.snapshot().frames;
    let out = run(&mut e, 60.0);
    let snap = h.snapshot();
    assert_eq!(snap.master_deck, Some(0));
    assert!((snap.decks[1].bpm - master_bpm).abs() < 0.05, "B bpm {}", snap.decks[1].bpm);
    // A's beats: A plays at 1.0x from 0 s; its beat n is at output time
    // 0.1 + n * 60/bpm (plus the 5 s already rendered).
    let t0 = (rendered as f64 - LIMITER_FRAMES) / f64::from(SR);
    let beat = 60.0 / master_bpm;
    let mut worst: f64 = 0.0;
    for f in onsets(&out) {
        let t = t0 + f as f64 / f64::from(SR);
        let phase = ((t - 0.1) / beat).rem_euclid(1.0);
        let err_ms = phase.min(1.0 - phase) * beat * 1000.0;
        worst = worst.max(err_ms);
    }
    worst
}

#[test]
fn sync_locks_phase_varispeed() {
    let worst = sync_test(false, 3.21);
    eprintln!("varispeed follower: worst {worst:.3} ms");
    assert!(worst < 0.5, "worst phase error {worst:.3} ms");
}

/// A big tempo difference (126 → 140 BPM, +11 %), as DJs have when mixing
/// across genres.
#[test]
fn sync_locks_phase_far_apart() {
    for keylock in [false, true] {
        let worst = sync_test_bpm(140.0, 126.0, keylock, 59.0);
        eprintln!("126 → 140 bpm, keylock {keylock}: worst {worst:.3} ms");
        assert!(worst < 0.5, "keylock {keylock}: worst phase error {worst:.3} ms");
    }
}

#[test]
fn sync_locks_phase_with_keylock() {
    let worst = sync_test(true, 7.77);
    eprintln!("keylock follower: worst {worst:.3} ms");
    assert!(worst < 0.5, "worst phase error {worst:.3} ms");
}

#[test]
fn loop_wraps_without_drift() {
    for keylock in [false, true] {
        loop_wraps(keylock);
    }
}

fn loop_wraps(keylock: bool) {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(125.0, 0.2, 120.0, 44_100), 10.0);
    if !keylock {
        press(&h, 0, Control::Keylock);
    }
    press(&h, 0, Control::Play);
    run(&mut e, 0.3);
    // 1-beat loop (sizes index 5), wraps 480 ms apart.
    for _ in 0..2 {
        press(&h, 0, Control::LoopSizeDown);
    }
    press(&h, 0, Control::LoopToggle);
    let out = run(&mut e, 60.0);
    let s = h.snapshot().decks[0];
    assert!(s.loop_active);
    assert!((s.loop_end_secs - s.loop_start_secs - 0.48).abs() < 1e-9);
    let on = onsets(&out);
    assert!(on.len() >= 120);
    let period = 0.48 * f64::from(SR);
    for w in on.windows(2) {
        let d = (w[1] - w[0]) as f64;
        assert!((d - period).abs() <= 1.0, "keylock {keylock}: interval {d}");
    }
    // Over 125 wraps the clicks stay on the same lattice.
    let first = on[0] as f64;
    let last = *on.last().unwrap() as f64;
    let wraps = ((last - first) / period).round();
    assert!((last - first - wraps * period).abs() <= 1.0);
}

#[test]
fn quantized_hotcue_keeps_phase() {
    let (h, mut e) = create(SR, 1024);
    let (audio, grid) = click_track(120.0, 0.0, 120.0, 48_000);
    load(&h, 0, 1, (audio, grid.clone()), 0.0);
    press(&h, 0, Control::Keylock);
    press(&h, 0, Control::Play);
    run(&mut e, 10.3);
    // Set hotcue 1 (snapped to a beat), play on, jump back to it.
    press(&h, 0, Control::Hotcue(1));
    run(&mut e, 5.37);
    let before = h.snapshot().decks[0].beat;
    ctl(&h, 0, Control::Hotcue(1), ControlValue::Press(true));
    run(&mut e, 0.01);
    ctl(&h, 0, Control::Hotcue(1), ControlValue::Press(false));
    let s = h.snapshot().decks[0];
    let cue = s.hotcues[0].unwrap().secs;
    assert!((grid.beat_at(cue) - grid.beat_at(cue).round()).abs() < 1e-9, "hotcue snapped to a beat");
    assert!(s.position_secs < 11.5, "jumped back: {}", s.position_secs);
    let after = s.beat;
    let dt_beats = 0.01 * 2.0 + (BLOCK as f64 / f64::from(SR)) * 2.0 * 2.0;
    let phase_diff = ((after - before).rem_euclid(1.0) - 0.0).min(1.0 - (after - before).rem_euclid(1.0));
    assert!(phase_diff < dt_beats + 0.01, "phase moved by {phase_diff} beats");
}

#[test]
fn cue_and_cup() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(120.0, 0.0, 60.0, 48_000), 0.0);
    run(&mut e, 0.01);
    // Seek to 5 s and press CUE while paused: sets the cue, previews while held.
    assert!(h.send(Command::SeekSecs { deck: 0, secs: 5.0 }).is_ok());
    ctl(&h, 0, Control::Cue, ControlValue::Press(true));
    run(&mut e, 1.0);
    let s = h.snapshot().decks[0];
    assert!(s.playing && s.cue_held);
    assert!((s.main_cue_secs - 5.0).abs() < 1e-9);
    ctl(&h, 0, Control::Cue, ControlValue::Press(false));
    run(&mut e, 0.1);
    let s = h.snapshot().decks[0];
    assert!(!s.playing);
    assert!((s.position_secs - 5.0).abs() < 1e-3);
    let mut set = false;
    h.poll(|ev| set |= matches!(ev, rille_engine::Event::MainCueSet { secs, .. } if (secs - 5.0).abs() < 1e-9));
    assert!(set);
    // CUP: plays from the cue on release.
    ctl(&h, 0, Control::Cup, ControlValue::Press(true));
    run(&mut e, 0.2);
    assert!(!h.snapshot().decks[0].playing);
    ctl(&h, 0, Control::Cup, ControlValue::Press(false));
    run(&mut e, 1.0);
    let s = h.snapshot().decks[0];
    assert!(s.playing);
    assert!((s.position_secs - 6.0).abs() < 0.02, "{}", s.position_secs);
}

#[test]
fn jump_start_keeps_transport_state() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(120.0, 0.0, 60.0, 48_000), 0.0);
    run(&mut e, 0.01);
    // Stopped: goes to 0 and stays stopped.
    assert!(h.send(Command::SeekSecs { deck: 0, secs: 20.0 }).is_ok());
    press(&h, 0, Control::JumpStart);
    run(&mut e, 0.2);
    let s = h.snapshot().decks[0];
    assert!(!s.playing);
    assert!(s.position_secs.abs() < 1e-3, "{}", s.position_secs);
    // Playing in a loop further on: leaves the loop and plays on from 0.
    assert!(h.send(Command::SeekSecs { deck: 0, secs: 20.0 }).is_ok());
    press(&h, 0, Control::Play);
    press(&h, 0, Control::LoopToggle);
    run(&mut e, 0.5);
    press(&h, 0, Control::JumpStart);
    run(&mut e, 1.0);
    let s = h.snapshot().decks[0];
    assert!(s.playing && !s.loop_active);
    assert!((s.position_secs - 1.0).abs() < 0.05, "{}", s.position_secs);
}

/// A starts playing before its grid arrives (analysis still running); then
/// B, already analyzed, is synced and started. A must lead, B follow A.
#[test]
fn late_grid_still_leads() {
    let (h, mut e) = create(SR, 1024);
    let (audio_a, grid_a) = click_track(123.0, 0.2, 120.0, 44_100);
    let track = LoadedTrack {
        id: 1,
        audio: audio_a,
        grid: None,
        main_cue_secs: 0.0,
        hotcues: [None; HOTCUES],
        auto_gain_db: 0.0,
    };
    assert!(h.send(Command::Load { deck: 0, track }).is_ok());
    press(&h, 0, Control::Play);
    run(&mut e, 1.0);
    assert!(h.send(Command::SetGrid { deck: 0, track_id: 1, grid: Some(grid_a) }).is_ok());
    run(&mut e, 0.5);
    load(&h, 1, 2, click_track(124.0, 0.3, 120.0, 44_100), 0.0);
    press(&h, 1, Control::Sync);
    press(&h, 1, Control::Play);
    run(&mut e, 3.0);
    let s = h.snapshot();
    assert_eq!(s.master_deck, Some(0), "A leads");
    assert!((s.decks[1].bpm - 123.0).abs() < 0.05, "B follows A: {}", s.decks[1].bpm);
    assert!((s.decks[0].bpm - 123.0).abs() < 0.01, "A keeps its tempo: {}", s.decks[0].bpm);
}

/// Sync pressed while nothing leads yet: the deck must still end up in
/// tempo with the deck that starts leading afterwards.
#[test]
fn sync_before_leader_exists() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 1, 2, click_track(124.0, 0.3, 120.0, 44_100), 0.0);
    press(&h, 1, Control::Sync);
    run(&mut e, 0.2);
    load(&h, 0, 1, click_track(126.0, 0.2, 120.0, 44_100), 0.0);
    press(&h, 0, Control::Play);
    run(&mut e, 0.5);
    press(&h, 1, Control::Play);
    run(&mut e, 3.0);
    let s = h.snapshot();
    assert_eq!(s.master_deck, Some(0));
    assert!((s.decks[1].bpm - 126.0).abs() < 0.05, "B follows A: {}", s.decks[1].bpm);
}

/// B (synced, analyzed) starts while A plays without a grid yet: B leads for
/// the moment, but once A's grid arrives, A takes over and B follows.
#[test]
fn unsynced_deck_takes_over_from_synced_leader() {
    let (h, mut e) = create(SR, 1024);
    let (audio_a, grid_a) = click_track(123.0, 0.2, 120.0, 44_100);
    let track = LoadedTrack {
        id: 1,
        audio: audio_a,
        grid: None,
        main_cue_secs: 0.0,
        hotcues: [None; HOTCUES],
        auto_gain_db: 0.0,
    };
    assert!(h.send(Command::Load { deck: 0, track }).is_ok());
    load(&h, 1, 2, click_track(124.0, 0.3, 120.0, 44_100), 0.0);
    press(&h, 0, Control::Play);
    press(&h, 1, Control::Sync);
    press(&h, 1, Control::Play);
    run(&mut e, 1.0);
    assert_eq!(h.snapshot().master_deck, Some(1), "B leads while A has no grid");
    assert!(h.send(Command::SetGrid { deck: 0, track_id: 1, grid: Some(grid_a) }).is_ok());
    run(&mut e, 4.0);
    let s = h.snapshot();
    assert_eq!(s.master_deck, Some(0), "A takes over");
    assert!((s.decks[0].bpm - 123.0).abs() < 0.01);
    assert!((s.decks[1].bpm - 123.0).abs() < 0.05, "B follows A: {}", s.decks[1].bpm);
    assert!(s.decks[1].phase_error.abs() < 0.01, "B in phase: {}", s.decks[1].phase_error);
}

/// Turning sync off keeps the synced tempo (the fader followed it).
#[test]
fn sync_off_keeps_tempo() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(125.0, 0.2, 120.0, 44_100), 0.0);
    load(&h, 1, 2, click_track(122.0, 0.3, 120.0, 44_100), 0.0);
    press(&h, 0, Control::Play);
    run(&mut e, 0.5);
    press(&h, 1, Control::Sync);
    press(&h, 1, Control::Play);
    run(&mut e, 1.0);
    press(&h, 1, Control::Sync);
    run(&mut e, 1.0);
    let s = h.snapshot();
    assert!(!s.decks[1].sync);
    assert!((s.decks[1].bpm - 125.0).abs() < 0.05, "B keeps 125: {}", s.decks[1].bpm);
}

/// Setting the master tempo moves the leading deck's tempo fader; a synced
/// deck follows.
#[test]
fn master_tempo_moves_the_leading_deck() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(136.0, 0.2, 120.0, 44_100), 0.0);
    load(&h, 1, 2, click_track(128.0, 0.3, 120.0, 44_100), 0.0);
    press(&h, 0, Control::Play);
    run(&mut e, 0.5);
    press(&h, 1, Control::Sync);
    press(&h, 1, Control::Play);
    run(&mut e, 0.5);
    assert!(h.send(Command::SetClockBpm(140.0)).is_ok());
    run(&mut e, 1.0);
    let s = h.snapshot();
    assert!((s.clock_bpm - 140.0).abs() < 0.05, "clock at 140: {}", s.clock_bpm);
    assert!((s.decks[0].bpm - 140.0).abs() < 0.05, "A plays 140: {}", s.decks[0].bpm);
    assert!((s.decks[1].bpm - 140.0).abs() < 0.05, "B follows 140: {}", s.decks[1].bpm);
}

/// TICK clicks on the grid beats of a silent track, exactly where the grid
/// puts them (at 1.0x and faster, with and without keylock), accent on bar 1.
#[test]
fn tick_clicks_on_the_grid() {
    for (keylock, fader) in [(false, 0.5), (true, 0.5), (true, 0.8)] {
        let (h, mut e) = create(SR, 1024);
        let silent = Arc::new(TrackAudio { sample_rate: 44_100, frames: vec![[0.0; 2]; 44_100 * 30] });
        let grid = Arc::new(BeatGrid::new(BeatMap::constant(0.25, 120.0).unwrap(), GridSource::Manual));
        load(&h, 0, 1, (silent, grid.clone()), 0.0);
        if !keylock {
            press(&h, 0, Control::Keylock);
        }
        ctl(&h, 0, Control::Tempo, ControlValue::Absolute(fader));
        press(&h, 0, Control::Tick);
        press(&h, 0, Control::Play);
        let out = run(&mut e, 8.0);
        let rate = h.snapshot().decks[0].rate;
        assert!(h.snapshot().decks[0].tick);
        // Click start: first sample of a burst after silence.
        let mut starts = Vec::new();
        let mut quiet = 0usize;
        for (i, f) in out.iter().enumerate() {
            if f[0].abs() > 1e-4 {
                if quiet > 1000 {
                    starts.push(i);
                }
                quiet = 0;
            } else {
                quiet += 1;
            }
        }
        assert!(starts.len() >= 14, "{} clicks", starts.len());
        for &s in &starts {
            let t = (s as f64 - LIMITER_FRAMES) / f64::from(SR) * rate;
            let beat = grid.beat_at(t);
            let err_ms = (beat - beat.round()) * 0.5 / rate * 1000.0;
            assert!(err_ms.abs() < 0.3, "keylock {keylock} fader {fader}: click {err_ms:.3} ms off the grid");
            // Accent (bar 1) is the higher tone: count zero crossings in 4 ms.
            let crossings = out[s..s + 192].windows(2).filter(|w| (w[0][0] >= 0.0) != (w[1][0] >= 0.0)).count();
            let accent = grid.is_downbeat(beat.round() as i64);
            assert_eq!(crossings > 16, accent, "beat {}: {crossings} crossings", beat.round());
        }
    }
}

/// Touch strip / jog rim on a paused deck moves the track (cueing).
#[test]
fn jog_moves_a_paused_deck() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(120.0, 0.0, 60.0, 48_000), 10.0);
    run(&mut e, 0.05);
    let before = h.snapshot().decks[0].position_secs;
    for _ in 0..10 {
        ctl(&h, 0, Control::Jog, ControlValue::Delta(0.01));
    }
    run(&mut e, 0.05);
    let after = h.snapshot().decks[0].position_secs;
    // 0.1 revolution = 0.18 s at 1.8 s per revolution.
    assert!((after - before - 0.18).abs() < 0.002, "moved {:.4} s", after - before);
    assert!(!h.snapshot().decks[0].playing);
}

/// Nudging a synced deck (touch strip) shifts it against the master and it
/// stays there: sync keeps the new offset and never jumps back.
#[test]
fn nudge_on_a_synced_deck_is_kept() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(124.0, 0.1, 400.0, 44_100), 0.0);
    load(&h, 1, 2, click_track(128.0, 0.37, 400.0, 48_000), 3.0);
    press(&h, 0, Control::Play);
    run(&mut e, 1.0);
    press(&h, 1, Control::Sync);
    press(&h, 1, Control::Play);
    run(&mut e, 2.0);
    let jumps = h.snapshot().decks[1].realigns;
    // Slide 0.04 revolution in small steps; positive deltas move the deck
    // ahead of the master.
    for _ in 0..8 {
        ctl(&h, 1, Control::Jog, ControlValue::Delta(0.005));
        run(&mut e, 0.02);
    }
    // Only deck B audible, then measure its clicks against A's beats.
    ctl(&h, 0, Control::Volume, ControlValue::Absolute(0.0));
    run(&mut e, 1.0);
    let rendered = h.snapshot().frames;
    let out = run(&mut e, 5.0);
    let t0 = (rendered as f64 - LIMITER_FRAMES) / f64::from(SR);
    let beat = 60.0 / 124.0;
    let offs: Vec<f64> = onsets(&out)
        .iter()
        .map(|&f| {
            let t = t0 + f as f64 / f64::from(SR);
            let phase = ((t - 0.1) / beat).rem_euclid(1.0);
            (if phase > 0.5 { phase - 1.0 } else { phase }) * beat * 1000.0
        })
        .collect();
    assert!(offs.len() >= 8);
    // 0.04 rev × 0.25 beat/rev = 0.01 beat = 4.84 ms at 124 bpm, ahead.
    let want = -0.01 * beat * 1000.0;
    for o in &offs {
        assert!((o - want).abs() < 0.6, "click at {o:+.2} ms, want {want:+.2} ms");
    }
    assert_eq!(h.snapshot().decks[1].realigns, jumps, "the nudge made sync jump");
    // The meter shows the offset.
    assert!(
        (h.snapshot().decks[1].phase_error + 0.01).abs() < 0.002,
        "phase meter {}",
        h.snapshot().decks[1].phase_error
    );
}

fn global(h: &EngineHandle, c: Control, v: ControlValue) {
    let target = ControlTarget::global(c);
    assert!(h.send(Command::Control(ControlEvent { target, value: v })).is_ok());
}

fn peak(out: &[[f32; 2]]) -> f32 {
    out.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()))
}

#[test]
fn filter_roll_slips_back_to_where_the_track_would_be() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(120.0, 0.0, 60.0, 48_000), 0.0);
    press(&h, 0, Control::Play);
    run(&mut e, 10.2);
    // Held at the centre: no roll yet.
    ctl(&h, 0, Control::FilterRoll, ControlValue::Press(true));
    run(&mut e, 0.1);
    assert!(!h.snapshot().decks[0].loop_active);
    // Turned 60 % of the way to the low-pass end: 1/4-beat roll (125 ms).
    ctl(&h, 0, Control::Filter, ControlValue::Absolute(0.2));
    run(&mut e, 1.0);
    let s = h.snapshot().decks[0];
    assert!(s.loop_active);
    assert!((s.loop_end_secs - s.loop_start_secs - 0.125).abs() < 1e-9, "{s:?}");
    assert!(s.loop_start_secs > 10.0 && s.loop_end_secs < 10.6, "rolls the beat it started on: {s:?}");
    // Further out: 1/16 beat.
    ctl(&h, 0, Control::Filter, ControlValue::Absolute(0.0));
    run(&mut e, 0.5);
    let s = h.snapshot().decks[0];
    assert!((s.loop_end_secs - s.loop_start_secs - 0.03125).abs() < 1e-9, "{s:?}");
    // Let go: no loop, playing on from where the track would be.
    ctl(&h, 0, Control::FilterRoll, ControlValue::Press(false));
    run(&mut e, 0.5);
    let s = h.snapshot().decks[0];
    assert!(s.playing && !s.loop_active);
    let want = 10.2 + 0.1 + 1.0 + 0.5 + 0.5;
    assert!((s.position_secs - want).abs() < 0.05, "at {} want {want}", s.position_secs);
}

#[test]
fn filter_roll_restores_a_loop_it_interrupted() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(120.0, 0.0, 60.0, 48_000), 0.0);
    press(&h, 0, Control::Play);
    run(&mut e, 10.2);
    press(&h, 0, Control::LoopToggle); // 4 beats = 2 s
    run(&mut e, 0.1);
    let s = h.snapshot().decks[0];
    let before = (s.loop_start_secs, s.loop_end_secs);
    assert!(s.loop_active && (before.1 - before.0 - 2.0).abs() < 1e-9, "{s:?}");
    ctl(&h, 0, Control::Filter, ControlValue::Absolute(0.9));
    ctl(&h, 0, Control::FilterRoll, ControlValue::Press(true));
    run(&mut e, 0.5);
    ctl(&h, 0, Control::FilterRoll, ControlValue::Press(false));
    run(&mut e, 3.0);
    let s = h.snapshot().decks[0];
    assert!(s.loop_active);
    assert_eq!((s.loop_start_secs, s.loop_end_secs), before, "{s:?}");
    assert!(s.position_secs >= s.loop_start_secs - 0.05 && s.position_secs <= s.loop_end_secs + 0.05, "{s:?}");
}

#[test]
fn eq_kills_hold_the_band_down() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(120.0, 0.0, 60.0, 48_000), 0.0);
    press(&h, 0, Control::Play);
    assert!(peak(&run(&mut e, 1.0)) > 0.3);
    for c in [Control::EqLoKill, Control::EqMidKill, Control::EqHiKill] {
        ctl(&h, 0, c, ControlValue::Press(true));
    }
    run(&mut e, 0.2);
    assert_eq!(h.snapshot().channels[0].kill, [true; 3]);
    assert!(peak(&run(&mut e, 1.0)) < 0.01, "all bands killed");
    assert_eq!(h.snapshot().channels[0].eq, [0.5; 3], "the knobs are untouched");
    ctl(&h, 0, Control::EqHiKill, ControlValue::Press(false));
    ctl(&h, 0, Control::EqMidKill, ControlValue::Press(false));
    ctl(&h, 0, Control::EqLoKill, ControlValue::Press(false));
    run(&mut e, 0.2);
    assert!(peak(&run(&mut e, 1.0)) > 0.3, "released");
}

#[test]
fn crossfader_reverse_and_curve() {
    let (h, mut e) = create(SR, 1024);
    load(&h, 0, 1, click_track(120.0, 0.0, 60.0, 48_000), 0.0);
    press(&h, 0, Control::Play);
    // Deck A is on the left; the fader all the way right mutes it.
    global(&h, Control::Crossfader, ControlValue::Absolute(1.0));
    run(&mut e, 0.2);
    assert!(peak(&run(&mut e, 1.0)) < 1e-3);
    global(&h, Control::CrossfaderReverse, ControlValue::Press(true));
    run(&mut e, 0.2);
    assert!(h.snapshot().crossfader_reverse);
    assert!(peak(&run(&mut e, 1.0)) > 0.3, "reversed: A is on the right");
    global(&h, Control::CrossfaderReverse, ControlValue::Press(false));
    run(&mut e, 0.2);
    assert!(peak(&run(&mut e, 1.0)) < 1e-3);
    // Scratch curve: A stays at full level until just before the far end.
    global(&h, Control::CrossfaderCurve, ControlValue::Absolute(1.0));
    global(&h, Control::Crossfader, ControlValue::Absolute(0.95));
    run(&mut e, 0.2);
    let s = h.snapshot();
    assert_eq!(s.crossfader_curve, 1.0);
    assert_eq!(s.crossfader_gain(0), 1.0);
    assert!(peak(&run(&mut e, 1.0)) > 0.3);
}

/// External mixer mode: each deck on its own output pair, with no fader,
/// crossfader or main mix in the way (a Xone:96 with decks C A B D on
/// channels 1-4, 12 outputs).
#[test]
fn external_mixer_routes_decks_to_their_pairs() {
    let (h, mut e) = create(SR, 1024);
    let tone = |level: f32| {
        let audio = Arc::new(TrackAudio { sample_rate: SR, frames: vec![[level, -level]; SR as usize * 4] });
        (audio, Arc::new(BeatGrid::new(BeatMap::constant(0.0, 120.0).unwrap(), GridSource::Manual)))
    };
    load(&h, 0, 1, tone(0.25), 0.0);
    load(&h, 1, 2, tone(0.5), 0.0);
    let outputs = Some([Some(1), Some(2), Some(0), Some(3)]);
    assert!(
        h.send(Command::Settings(rille_engine::Settings { external_outputs: outputs, ..Default::default() })).is_ok()
    );
    // The internal fader and crossfader would silence deck A.
    ctl(&h, 0, Control::Volume, ControlValue::Absolute(0.0));
    let xfader =
        ControlEvent { target: ControlTarget::global(Control::Crossfader), value: ControlValue::Absolute(1.0) };
    assert!(h.send(Command::Control(xfader)).is_ok());
    press(&h, 0, Control::Play);
    press(&h, 1, Control::Play);
    const CH: usize = 12;
    let mut out = vec![0.0f32; BLOCK * CH];
    for _ in 0..40 {
        assert_no_alloc(|| e.process_interleaved(&mut out, CH));
    }
    let last = &out[(BLOCK - 1) * CH..];
    let expect = [0.0, 0.0, 0.25, -0.25, 0.5, -0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    for (got, want) in last.iter().zip(expect) {
        assert!((got - want).abs() < 1e-3, "{last:?}");
    }
    assert_eq!(e.render(BLOCK).0.iter().map(|f| f[0].abs()).fold(0.0, f32::max), 0.0, "no main mix");

    // Back to the internal mixer: the main mix (deck B) on 1/2.
    assert!(h.send(Command::Settings(rille_engine::Settings::default())).is_ok());
    for _ in 0..40 {
        e.process_interleaved(&mut out, CH);
    }
    let last = &out[(BLOCK - 1) * CH..];
    assert!(last[0] > 0.1 && last[4..].iter().all(|v| *v == 0.0), "{last:?}");
}
