//! Audio output through cpal: PipeWire natively where it runs, then JACK
//! (when a server is running), then plain ALSA.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::{Engine, EngineHandle, create};

/// A running output stream. Dropping it stops audio.
pub struct AudioOutput {
    _stream: cpal::Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Requested buffer size in frames, if any.
    pub buffer_frames: Option<u32>,
    /// Real-time priority of the audio thread once granted (0 = none yet or
    /// refused), see `realtime`.
    pub realtime_priority: Arc<AtomicI64>,
    /// The device is an external DJ mixer's sound card (see
    /// [`is_external_mixer`]) with outputs for at least two of its
    /// channels, so the decks can go to them.
    pub external_mixer: bool,
}

/// DJ controllers with a built-in sound card whose 4-channel output carries
/// the main mix on 1/2 and the headphones on 3/4, as the engine renders it.
/// With no device configured, or one of the card's stereo outputs chosen, the
/// 4-channel output is used. Under PipeWire the Z1's card is split into "Line
/// Out" and "Headphone Out" sinks; its 4-channel node is "Traktor Kontrol Z1 0".
/// The Akai AMX's card is "AMX" (MASTER OUT 1/2, headphones 3/4).
const CONTROLLER_OUTPUTS: &[&str] = &["Traktor Kontrol Z1", "AMX"];

fn is_controller_output(name: &str, channels: u16) -> bool {
    channels >= 4 && CONTROLLER_OUTPUTS.iter().any(|c| name.starts_with(c))
}

/// Hardware DJ mixers with a USB sound card that has an input per mixer
/// channel, matched on the device name with case, spaces and punctuation
/// ignored. The Allen & Heath Xone:96 has two such cards (USB 1 and USB 2),
/// 12 outputs each: 1/2 … 7/8 feed channels 1-4, 9/10 and 11/12 channels A
/// and B. Under PipeWire its card needs the "Pro Audio" (or multichannel)
/// profile; the stereo profile only reaches channel 1.
const MIXER_OUTPUTS: &[&str] = &["xone96"];

/// Whether `name` is an external mixer's sound card (any of its outputs).
pub fn is_external_mixer(name: &str) -> bool {
    let key: String = name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    MIXER_OUTPUTS.iter().any(|m| key.contains(m))
}

/// An external mixer output with a stereo pair for at least two channels.
fn is_mixer_output(name: &str, channels: u16) -> bool {
    channels >= 4 && is_external_mixer(name)
}

#[derive(Clone, Debug, Default)]
pub struct AudioConfig {
    /// Output device name; `None` for a DJ controller's sound card if one is
    /// connected (see `CONTROLLER_OUTPUTS`), else an external mixer's (see
    /// `MIXER_OUTPUTS`), else the system default. Naming any output of such
    /// a card selects its multichannel output.
    pub device: Option<String>,
    /// Buffer size in frames (e.g. 256); `None` for the device default.
    pub buffer_frames: Option<u32>,
}

/// Hosts to try, best first.
fn hosts() -> Vec<cpal::Host> {
    let mut ids: Vec<cpal::HostId> = Vec::new();
    for want in ["PipeWire", "JACK"] {
        if let Some(id) = cpal::available_hosts().into_iter().find(|h| h.name() == want) {
            ids.push(id);
        }
    }
    ids.push(cpal::default_host().id());
    ids.dedup();
    ids.into_iter().filter_map(|id| cpal::host_from_id(id).ok()).collect()
}

/// Names of the available output devices.
pub fn output_devices() -> Vec<String> {
    let Some(host) = hosts().into_iter().next() else { return Vec::new() };
    host.output_devices()
        .map(|devs| devs.filter_map(|d| d.description().ok().map(|n| n.name().to_string())).collect())
        .unwrap_or_default()
}

/// Opens the output device, creates an engine at its sample rate and starts
/// the stream. Returns the handle for the app and the running stream.
pub fn start(config: &AudioConfig) -> Result<(EngineHandle, AudioOutput), String> {
    let mut errors = Vec::new();
    for host in hosts() {
        match start_on(&host, config) {
            Ok(r) => return Ok(r),
            Err(e) => errors.push(format!("{}: {e}", host.id().name())),
        }
    }
    Err(errors.join("; "))
}

fn start_on(host: &cpal::Host, config: &AudioConfig) -> Result<(EngineHandle, AudioOutput), String> {
    let wants_controller =
        config.device.as_deref().is_none_or(|name| CONTROLLER_OUTPUTS.iter().any(|c| name.starts_with(c)));
    if wants_controller && let Some(r) = controller_output(host).and_then(|d| open(host, d, config).ok()) {
        return Ok(r);
    }
    let wants_mixer = config.device.as_deref().is_none_or(is_external_mixer);
    if wants_mixer && let Some(r) = mixer_output(host).and_then(|d| open(host, d, config).ok()) {
        return Ok(r);
    }
    let device = match &config.device {
        Some(name) => host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.description().is_ok_and(|n| n.name() == name))
            .ok_or_else(|| format!("output device '{name}' not found"))?,
        None => host.default_output_device().ok_or("no output device")?,
    };
    open(host, device, config)
}

fn controller_output(host: &cpal::Host) -> Option<cpal::Device> {
    host.output_devices().ok()?.find(|d| {
        let channels = d.default_output_config().map_or(0, |c| c.channels());
        d.description().is_ok_and(|n| is_controller_output(n.name(), channels))
    })
}

/// The external mixer output with the most channels.
fn mixer_output(host: &cpal::Host) -> Option<cpal::Device> {
    let channels = |d: &cpal::Device| d.default_output_config().map_or(0, |c| c.channels());
    host.output_devices()
        .ok()?
        .filter(|d| d.description().is_ok_and(|n| is_mixer_output(n.name(), channels(d))))
        .max_by_key(channels)
}

fn open(host: &cpal::Host, device: cpal::Device, config: &AudioConfig) -> Result<(EngineHandle, AudioOutput), String> {
    let device_name = device.description().map(|n| n.name().to_string()).unwrap_or_default();
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let mut stream_config: cpal::StreamConfig = supported.config();
    if let Some(frames) = config.buffer_frames {
        stream_config.buffer_size = cpal::BufferSize::Fixed(frames);
    }
    let sample_rate = stream_config.sample_rate;
    let channels = stream_config.channels;
    let (handle, engine) = create(sample_rate, 1024);
    // The callback reports its thread once; a helper thread then asks for
    // real-time priority for it.
    let audio_tid = Arc::new(AtomicI64::new(0));
    let xruns = handle.xruns.clone();
    let stream = build(&device, &stream_config, supported.sample_format(), engine, audio_tid.clone(), xruns)?;
    stream.play().map_err(|e| e.to_string())?;
    let realtime_priority = Arc::new(AtomicI64::new(0));
    let granted = realtime_priority.clone();
    // Only Linux has a way to ask for it (RealtimeKit).
    if cfg!(target_os = "linux") {
        let _ = std::thread::Builder::new().name("audio-rt".into()).spawn(move || {
            for _ in 0..300 {
                let tid = audio_tid.load(Ordering::Acquire);
                if tid != 0 {
                    match crate::realtime::promote(tid) {
                        Ok(p) => granted.store(p, Ordering::Release),
                        Err(e) => eprintln!("audio thread stays at normal priority: {e}"),
                    }
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
    }
    // A mixer card on a stereo profile reaches one channel: mix internally.
    let external_mixer = is_mixer_output(&device_name, channels);
    let device_name = format!("{device_name} ({})", host.id().name());
    Ok((
        handle,
        AudioOutput {
            _stream: stream,
            device_name,
            sample_rate,
            channels,
            buffer_frames: config.buffer_frames,
            realtime_priority,
            external_mixer,
        },
    ))
}

fn build(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    mut engine: Engine,
    audio_tid: Arc<AtomicI64>,
    xruns: Arc<AtomicU64>,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels);
    // Once, from inside the callback: which thread runs the audio.
    let mut reported = false;
    let mut report = move || {
        if !reported {
            reported = true;
            audio_tid.store(crate::realtime::current_tid(), Ordering::Release);
        }
    };
    // Dropouts are only counted (the app shows them): this may run on the
    // audio thread, and printing each one would add to the problem.
    let err = move |e: cpal::Error| {
        if e.kind() == cpal::ErrorKind::Xrun {
            xruns.fetch_add(1, Ordering::Relaxed);
        } else {
            eprintln!("audio stream: {e}");
        }
    };
    match format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                *config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    report();
                    engine.process_interleaved(data, channels);
                },
                err,
                None,
            )
            .map_err(|e| e.to_string()),
        cpal::SampleFormat::I16 => {
            let mut scratch = vec![0.0f32; 65_536];
            device
                .build_output_stream(
                    *config,
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        report();
                        let buf = &mut scratch[..data.len().min(65_536)];
                        engine.process_interleaved(buf, channels);
                        for (d, s) in data.iter_mut().zip(buf.iter()) {
                            *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                    },
                    err,
                    None,
                )
                .map_err(|e| e.to_string())
        }
        cpal::SampleFormat::I32 => {
            let mut scratch = vec![0.0f32; 65_536];
            device
                .build_output_stream(
                    *config,
                    move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                        report();
                        let buf = &mut scratch[..data.len().min(65_536)];
                        engine.process_interleaved(buf, channels);
                        for (d, s) in data.iter_mut().zip(buf.iter()) {
                            *d = (f64::from(s.clamp(-1.0, 1.0)) * 2_147_483_647.0) as i32;
                        }
                    },
                    err,
                    None,
                )
                .map_err(|e| e.to_string())
        }
        f => Err(format!("unsupported sample format {f:?}")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn controller_outputs() {
        use super::is_controller_output;
        assert!(is_controller_output("Traktor Kontrol Z1 0", 4));
        assert!(!is_controller_output("Traktor Kontrol Z1 Line Out", 2), "a stereo split sink has no cue");
        assert!(!is_controller_output("Starship/Matisse HD Audio Controller Analoges Stereo", 4));
        assert!(is_controller_output("AMX 0", 4));
        assert!(!is_controller_output("AMX Analog Stereo", 2));
    }

    #[test]
    fn mixer_outputs() {
        use super::{is_external_mixer, is_mixer_output};
        for name in ["XONE:96 Pro", "Xone:96 USB 1 Multichannel", "XONE 96", "hw:CARD=XONE96,DEV=0"] {
            assert!(is_external_mixer(name), "{name}");
        }
        assert!(is_mixer_output("XONE:96 Pro", 12));
        assert!(!is_mixer_output("XONE:96 Analog Stereo", 2), "a stereo profile reaches one channel only");
        assert!(!is_external_mixer("Allen & Heath Xone:K2"));
        assert!(!is_external_mixer("Xone:92"));
    }
}
