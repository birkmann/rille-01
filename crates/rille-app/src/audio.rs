//! Starts the audio output on its own thread (the stream lives there), or a
//! silent clock when no device is available so the app still works.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rille_engine::EngineHandle;
use rille_engine::backend::{self, AudioConfig};

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
}

pub struct AudioRunner {
    stop: mpsc::Sender<()>,
}

impl Drop for AudioRunner {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

/// Opens the device (or falls back to a silent clock) and returns the engine
/// handle, a status, and a guard that stops audio when dropped.
pub fn start(cfg: &AudioConfig, allow_device: bool) -> (Arc<EngineHandle>, AudioStatus, AudioRunner) {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    if allow_device {
        let (ready_tx, ready_rx) = mpsc::channel();
        let cfg = cfg.clone();
        std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || match backend::start(&cfg) {
                Ok((handle, out)) => {
                    let status = AudioStatus {
                        device: out.device_name.clone(),
                        sample_rate: out.sample_rate,
                        channels: out.channels,
                        buffer_frames: out.buffer_frames,
                        external_mixer: out.external_mixer,
                        error: None,
                    };
                    let _ = ready_tx.send(Ok((handle, status)));
                    let _ = stop_rx.recv();
                    drop(out);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err((e, stop_rx)));
                }
            })
            .expect("spawn audio thread");
        match ready_rx.recv() {
            Ok(Ok((handle, status))) => return (Arc::new(handle), status, AudioRunner { stop: stop_tx }),
            Ok(Err((e, stop_rx))) => return silent(Some(e), stop_tx, stop_rx),
            Err(_) => {
                let (tx, rx) = mpsc::channel();
                return silent(Some("audio thread failed".into()), tx, rx);
            }
        }
    }
    silent(None, stop_tx, stop_rx)
}

/// Renders in real time without output (no device, tests, CI).
fn silent(
    error: Option<String>,
    stop_tx: mpsc::Sender<()>,
    stop_rx: mpsc::Receiver<()>,
) -> (Arc<EngineHandle>, AudioStatus, AudioRunner) {
    const SR: u32 = 48_000;
    const BLOCK: usize = 512;
    let (handle, mut engine) = rille_engine::create(SR, BLOCK);
    std::thread::Builder::new()
        .name("audio-silent".into())
        .spawn(move || {
            let block = Duration::from_secs_f64(BLOCK as f64 / f64::from(SR));
            let mut next = Instant::now();
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                engine.render(BLOCK);
                next += block;
                if let Some(wait) = next.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                } else {
                    next = Instant::now();
                }
            }
        })
        .expect("spawn silent clock");
    let status = AudioStatus {
        device: "none (silent)".into(),
        sample_rate: SR,
        channels: 2,
        buffer_frames: Some(BLOCK as u32),
        external_mixer: false,
        error,
    };
    (Arc::new(handle), status, AudioRunner { stop: stop_tx })
}
