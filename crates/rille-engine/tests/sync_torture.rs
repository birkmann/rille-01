//! Sync under abuse: random seeks, jumps, loops, tempo moves, keylock,
//! scratching, nudges and master handovers on two synced click tracks. Deck
//! A clicks on the left channel only, deck B on the right, so the master
//! output shows both. Shortly after every operation, each click of one deck
//! must sound within 1 ms of a click of the other.

use std::sync::Arc;

use rille_core::{BeatGrid, BeatMap, Control, ControlEvent, ControlTarget, ControlValue, GridSource};
use rille_engine::{Command, Engine, EngineHandle, HOTCUES, LoadedTrack, TrackAudio, create};

const SR: u32 = 48_000;
const BLOCK: usize = 256;

/// A 1 ms pulse at every beat on one channel only.
fn click_track(bpm: f64, first_beat: f64, secs: f64, sr: u32, channel: usize) -> (Arc<TrackAudio>, Arc<BeatGrid>) {
    let n = (secs * f64::from(sr)) as usize;
    let mut frames = vec![[0.0f32; 2]; n];
    let beat = 60.0 / bpm;
    let mut t = first_beat;
    while t < secs - 0.01 {
        let s = (t * f64::from(sr)).round() as usize;
        for k in 0..(f64::from(sr) * 0.001) as usize {
            if let Some(f) = frames.get_mut(s + k) {
                f[channel] = 0.9;
            }
        }
        t += beat;
    }
    let grid = BeatGrid::new(BeatMap::constant(first_beat, bpm).unwrap(), GridSource::Manual);
    (Arc::new(TrackAudio { sample_rate: sr, frames }), Arc::new(grid))
}

fn load(h: &EngineHandle, deck: u8, id: u64, (audio, grid): (Arc<TrackAudio>, Arc<BeatGrid>)) {
    let track =
        LoadedTrack { id, audio, grid: Some(grid), main_cue_secs: 0.0, hotcues: [None; HOTCUES], auto_gain_db: 0.0 };
    assert!(h.send(Command::Load { deck, track }).is_ok());
}

fn ctl(h: &EngineHandle, deck: u8, c: Control, v: ControlValue) {
    assert!(h.send(Command::Control(ControlEvent { target: ControlTarget::deck(deck, c), value: v })).is_ok());
}

fn global(h: &EngineHandle, c: Control, v: ControlValue) {
    assert!(h.send(Command::Control(ControlEvent { target: ControlTarget::global(c), value: v })).is_ok());
}

fn press(h: &EngineHandle, deck: u8, c: Control) {
    ctl(h, deck, c, ControlValue::Press(true));
    ctl(h, deck, c, ControlValue::Press(false));
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn render(e: &mut Engine, secs: f64, out: &mut Vec<[f32; 2]>) {
    let blocks = (secs * f64::from(SR) / BLOCK as f64).ceil() as usize;
    for _ in 0..blocks {
        let (m, _) = e.render(BLOCK);
        out.extend_from_slice(m);
    }
}

/// Frames where a click starts on `channel`.
fn onsets(out: &[[f32; 2]], channel: usize) -> Vec<usize> {
    let mut v = Vec::new();
    let mut quiet = 0usize;
    for (i, f) in out.iter().enumerate() {
        if f[channel].abs() > 0.3 {
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

/// One scripted operation; returns its name and how long sync may take to
/// settle afterwards (s).
fn operation(h: &EngineHandle, rng: &mut Rng, e: &mut Engine, out: &mut Vec<[f32; 2]>) -> (&'static str, f64) {
    let snap = h.snapshot();
    let master = snap.master_deck.unwrap_or(0);
    let follower = 1 - master;
    let deck = if rng.below(3) == 0 { follower } else { master };
    match rng.below(13) {
        0 => {
            let secs = 20.0 + rng.unit() * 300.0;
            assert!(h.send(Command::SeekSecs { deck, secs }).is_ok());
            ("seek", 0.15)
        }
        1 => {
            press(h, deck, if rng.below(2) == 0 { Control::BeatjumpBack } else { Control::BeatjumpForward });
            ("beatjump", 0.15)
        }
        2 => {
            // Hotcue 1: the first press stores it, later ones jump there.
            press(h, deck, Control::Hotcue(1));
            ("hotcue", 0.15)
        }
        3 => {
            global(h, Control::Quantize, ControlValue::Press(true));
            global(h, Control::Quantize, ControlValue::Press(false));
            ("quantize toggle", 0.0)
        }
        4 => {
            press(h, deck, Control::LoopToggle);
            ("loop toggle", 0.15)
        }
        5 => {
            ctl(h, deck, Control::Tempo, ControlValue::Absolute(0.3 + 0.4 * rng.unit() as f32));
            ("tempo move", 0.15)
        }
        6 => {
            press(h, deck, Control::Keylock);
            ("keylock", 0.15)
        }
        7 => {
            assert!(h.send(Command::SetMaster(Some(follower))).is_ok());
            ("master handover", 0.15)
        }
        8 => {
            // Stop and restart after a moment.
            press(h, deck, Control::Play);
            render(e, 0.3 + rng.unit(), out);
            press(h, deck, Control::Play);
            ("stop/start", 0.15)
        }
        9 | 10 => {
            // Scratch: touch, move back and forth, release.
            ctl(h, deck, Control::JogTouch, ControlValue::Press(true));
            for _ in 0..20 {
                ctl(h, deck, Control::Jog, ControlValue::Delta(if rng.below(2) == 0 { 0.05 } else { -0.05 }));
                render(e, 0.02, out);
            }
            ctl(h, deck, Control::JogTouch, ControlValue::Press(false));
            ("scratch", 0.15)
        }
        11 => {
            // Nudge a synced follower: pushed while held, glides back after.
            let c = if rng.below(2) == 0 { Control::TempoBendUp } else { Control::TempoBendDown };
            ctl(h, follower, c, ControlValue::Press(true));
            render(e, 0.2 + 0.3 * rng.unit(), out);
            ctl(h, follower, c, ControlValue::Press(false));
            ("follower nudge", 0.8)
        }
        _ => {
            press(h, deck, Control::LoopHalve);
            ("loop halve", 0.15)
        }
    }
}

#[test]
fn sync_survives_random_operations() {
    // RILLE_TORTURE_SEEDS=n runs seeds 1..=n instead of the quick default set.
    let seeds: Vec<u64> = match std::env::var("RILLE_TORTURE_SEEDS").ok().and_then(|v| v.parse().ok()) {
        Some(n) => (1..=n).collect(),
        None => vec![1, 7, 42],
    };
    for seed in seeds {
        let (h, mut e) = create(SR, 1024);
        load(&h, 0, 1, click_track(124.0, 0.1, 400.0, 44_100, 0));
        load(&h, 1, 2, click_track(128.0, 0.37, 400.0, 48_000, 1));
        let mut out: Vec<[f32; 2]> = Vec::new();
        render(&mut e, 0.1, &mut out);
        press(&h, 0, Control::Play);
        press(&h, 1, Control::Sync);
        press(&h, 1, Control::Play);
        press(&h, 0, Control::Sync);
        render(&mut e, 2.0, &mut out);

        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        // (check from, check to, operation) in output frames.
        let mut windows: Vec<(usize, usize, &'static str)> = Vec::new();
        for _ in 0..40 {
            let before = h.snapshot();
            let (name, settle) = operation(&h, &mut rng, &mut e, &mut out);
            if std::env::var_os("RILLE_TORTURE_LOG").is_some() {
                let s = h.snapshot();
                eprintln!(
                    "{:7.2} s {name:16} master {:?}->{:?} keylock {:?} playing {:?} loop {:?}",
                    out.len() as f64 / f64::from(SR),
                    before.master_deck,
                    s.master_deck,
                    [s.decks[0].keylock, s.decks[1].keylock],
                    [s.decks[0].playing, s.decks[1].playing],
                    [s.decks[0].loop_active, s.decks[1].loop_active],
                );
            }
            let op_end = out.len();
            let hold = 1.2 + rng.unit() * 1.5;
            render(&mut e, hold, &mut out);
            // Loops shorter than a beat hide the master's beats: skip.
            let s = h.snapshot();
            let tiny_loop = s.decks.iter().any(|d| d.loop_active && d.loop_end_secs - d.loop_start_secs < 0.45);
            let both_play = s.decks[0].playing && s.decks[1].playing;
            if both_play && !tiny_loop {
                let from = op_end + ((settle + 0.05) * f64::from(SR)) as usize;
                windows.push((from, out.len() - (0.03 * f64::from(SR)) as usize, name));
            }
        }

        let a = onsets(&out, 0);
        let b = onsets(&out, 1);
        let mut checked = 0;
        let mut bad = Vec::new();
        for &(from, to, name) in &windows {
            for &fb in b.iter().filter(|&&f| f >= from && f < to) {
                let nearest = a.iter().map(|&fa| (fa as f64 - fb as f64).abs()).fold(f64::MAX, f64::min);
                let ms = nearest / f64::from(SR) * 1000.0;
                if ms > 1.0 {
                    bad.push(format!("{:.3} s after {name}: {ms:.2} ms", fb as f64 / f64::from(SR)));
                }
                checked += 1;
            }
        }
        assert!(bad.is_empty(), "seed {seed}: B clicks off A's:\n{}", bad.join("\n"));
        eprintln!("seed {seed}: {} windows, {checked} clicks checked", windows.len());
        assert!(checked > 50, "seed {seed}: only {checked} clicks checked");
    }
}
