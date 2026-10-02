//! Starts the audio output on its own thread (the stream lives there), or a
//! silent clock when no device is available so the app still works.
//!
//! The thread watches the device: when it goes away (a USB cable pulled, the
//! sound server restarting) the same engine keeps running on a silent
//! clock, so the decks play on, and the device is opened again as soon as it
//! is back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rille_engine::EngineHandle;
use rille_engine::backend::{self, AudioConfig, AudioOutput, SharedEngine, is_external_mixer};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioStatus {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_frames: Option<u32>,
    /// The output is a hardware DJ mixer's sound card (see
    /// `rille_engine::backend::is_external_mixer`).
    pub external_mixer: bool,
    /// Set when the device failed and the silent clock runs instead.
    pub error: Option<String>,
    /// Without a device, the device being waited for (it is opened again
    /// when it comes back).
    pub waiting_for: Option<String>,
}

/// Sample rate of the silent clock, and of the engine when audio starts
/// without a device.
const SILENT_RATE: u32 = 48_000;
/// How often a missing device is looked for.
const RETRY: Duration = Duration::from_secs(1);
/// A stream whose callback has not run for this long is taken as lost.
/// PipeWire reports a vanished device only once the stream is dropped.
const STALLED: Duration = Duration::from_secs(1);

/// Stops audio (and joins its thread) when dropped.
pub struct AudioRunner {
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for AudioRunner {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Opens the device (or falls back to a silent clock) and returns the engine
/// handle, a status, and a guard that stops audio when dropped. While the
/// device is missing it is looked for again; `on_change` gets the status
/// whenever it is lost or back.
pub fn start(
    cfg: &AudioConfig,
    allow_device: bool,
    on_change: impl Fn(AudioStatus) + Send + 'static,
) -> (Arc<EngineHandle>, AudioStatus, AudioRunner) {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    if !allow_device {
        let (handle, engine) = silent_engine();
        let clock = SilentClock::start(engine);
        let thread = std::thread::Builder::new()
            .name("audio-silent-guard".into())
            .spawn(move || {
                let _ = stop_rx.recv();
                drop(clock);
            })
            .expect("spawn audio thread");
        let status = silent_status(None, None);
        return (Arc::new(handle), status, AudioRunner { stop: stop_tx, thread: Some(thread) });
    }
    let (ready_tx, ready_rx) = mpsc::channel();
    let cfg = cfg.clone();
    let thread = std::thread::Builder::new()
        .name("audio-output".into())
        .spawn(move || {
            let mut watch = match backend::start(&cfg) {
                Ok((handle, out)) => {
                    let _ = ready_tx.send((handle, status(&out)));
                    Watch::playing(&cfg, out)
                }
                Err(e) => {
                    // Wait for the configured device with an engine of our own.
                    let (handle, engine) = silent_engine();
                    let waiting = cfg.device.clone().unwrap_or_else(|| "an audio output".into());
                    let _ = ready_tx.send((handle, silent_status(Some(e), Some(waiting))));
                    Watch::waiting(cfg.clone(), engine, None)
                }
            };
            while let Err(RecvTimeoutError::Timeout) = stop_rx.recv_timeout(Duration::from_millis(200)) {
                if let Some(s) = watch.check() {
                    on_change(s);
                }
            }
        })
        .expect("spawn audio thread");
    let (handle, status) = ready_rx.recv().expect("audio thread starts");
    (Arc::new(handle), status, AudioRunner { stop: stop_tx, thread: Some(thread) })
}

/// The audio thread's state: playing on a device, or waiting for it.
struct Watch {
    engine: SharedEngine,
    /// What to open again: the configured device, or the controller or
    /// mixer card that was found for "automatic".
    cfg: AudioConfig,
    host: Option<String>,
    state: State,
}

enum State {
    Playing {
        out: AudioOutput,
        seen: u64,
        since: Instant,
    },
    Waiting {
        _clock: SilentClock,
        next: Instant,
        last_error: Option<String>,
    },
    /// Between the two, so the stream is gone before the clock starts.
    Stopped,
}

impl Watch {
    fn playing(cfg: &AudioConfig, out: AudioOutput) -> Self {
        // "Automatic" found a controller or mixer card: wait for that one
        // rather than falling back to the laptop speakers.
        let mut cfg = cfg.clone();
        if cfg.device.is_none() && (backend::is_controller_output_name(&out.device) || is_external_mixer(&out.device)) {
            cfg.device = Some(out.device.clone());
        }
        let host = Some(out.host.clone());
        let engine = out.engine.clone();
        Self { engine, cfg, host, state: State::Playing { seen: out.callbacks(), since: Instant::now(), out } }
    }

    fn waiting(cfg: AudioConfig, engine: SharedEngine, host: Option<String>) -> Self {
        let state = State::Waiting {
            _clock: SilentClock::start(engine.clone()),
            next: Instant::now() + RETRY,
            last_error: None,
        };
        Self { engine, cfg, host, state }
    }

    /// Notices a lost device or one that came back; returns the new status
    /// if either happened.
    fn check(&mut self) -> Option<AudioStatus> {
        match &mut self.state {
            State::Playing { out, seen, since } => {
                let n = out.callbacks();
                if n != *seen {
                    (*seen, *since) = (n, Instant::now());
                }
                if !out.is_lost() && since.elapsed() < STALLED {
                    return None;
                }
                let (device, external_mixer) = (out.device.clone(), out.external_mixer);
                eprintln!("audio output lost: {device}; playing on silently until it is back");
                // Dropping the stream before the clock takes over: never two
                // renderers at once.
                self.state = State::Stopped;
                self.state = State::Waiting {
                    _clock: SilentClock::start(self.engine.clone()),
                    next: Instant::now() + RETRY,
                    last_error: None,
                };
                let error = format!("{device} disconnected");
                // The decks stay routed as they were until it is back.
                Some(AudioStatus { external_mixer, ..silent_status_at(&self.engine, Some(error), Some(device)) })
            }
            State::Waiting { next, last_error, .. } => {
                if Instant::now() < *next {
                    return None;
                }
                // The clock keeps time while the device opens; the two overlap
                // for at most one buffer.
                match backend::resume(&self.cfg, self.engine.clone(), self.host.as_deref()) {
                    Ok(out) => {
                        eprintln!("audio output back: {}", out.device_name);
                        let s = status(&out);
                        self.state = State::Playing { seen: out.callbacks(), since: Instant::now(), out };
                        Some(s)
                    }
                    Err(e) => {
                        if last_error.as_ref() != Some(&e) {
                            eprintln!("audio output still missing: {e}");
                            *last_error = Some(e);
                        }
                        *next = Instant::now() + RETRY;
                        None
                    }
                }
            }
            State::Stopped => None,
        }
    }
}

fn status(out: &AudioOutput) -> AudioStatus {
    AudioStatus {
        device: out.device_name.clone(),
        sample_rate: out.sample_rate,
        channels: out.channels,
        buffer_frames: out.buffer_frames,
        external_mixer: out.external_mixer,
        error: None,
        waiting_for: None,
    }
}

fn silent_engine() -> (EngineHandle, SharedEngine) {
    let (handle, engine) = rille_engine::create(SILENT_RATE, BLOCK);
    let shared = SharedEngine::new(&handle, engine);
    (handle, shared)
}

fn silent_status(error: Option<String>, waiting_for: Option<String>) -> AudioStatus {
    AudioStatus {
        device: "none (silent)".into(),
        sample_rate: SILENT_RATE,
        channels: 2,
        buffer_frames: Some(BLOCK as u32),
        external_mixer: false,
        error,
        waiting_for,
    }
}

/// Like [`silent_status`], at the engine's sample rate.
fn silent_status_at(engine: &SharedEngine, error: Option<String>, waiting_for: Option<String>) -> AudioStatus {
    AudioStatus { sample_rate: engine.lock().sample_rate(), ..silent_status(error, waiting_for) }
}

const BLOCK: usize = 512;

/// Renders the engine in real time without output (no device, tests, CI,
/// or while the device is away). Stops when dropped.
struct SilentClock {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SilentClock {
    fn start(engine: SharedEngine) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("audio-silent".into())
            .spawn(move || {
                let sr = engine.lock().sample_rate();
                let block = Duration::from_secs_f64(BLOCK as f64 / f64::from(sr));
                let mut next = Instant::now();
                while !flag.load(Ordering::Relaxed) {
                    engine.lock().render(BLOCK);
                    next += block;
                    if let Some(wait) = next.checked_duration_since(Instant::now()) {
                        std::thread::sleep(wait);
                    } else {
                        next = Instant::now();
                    }
                }
            })
            .expect("spawn silent clock");
        Self { stop, thread: Some(thread) }
    }
}

impl Drop for SilentClock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
