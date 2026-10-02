//! Decodes audio files to stereo `f32` at the file's native sample rate.
//!
//! Playback and analysis both decode through [`decode_file`], so sample
//! positions (and therefore beatgrids) mean exactly the same thing in both.
//! Encoder delay and padding (MP3/AAC priming) are removed by the decoder.

use std::fmt;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

/// Fully decoded track: interleaved stereo frames.
#[derive(Clone, Debug, Default)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub frames: Vec<[f32; 2]>,
}

impl DecodedAudio {
    pub fn duration_secs(&self) -> f64 {
        self.frames.len() as f64 / f64::from(self.sample_rate.max(1))
    }

    /// Mono mixdown for analysis.
    pub fn mono(&self) -> Vec<f32> {
        self.frames.iter().map(|[l, r]| 0.5 * (l + r)).collect()
    }
}

#[derive(Debug)]
pub enum DecodeError {
    Io(std::io::Error),
    Unsupported(String),
    NoAudioTrack,
    Cancelled,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Unsupported(s) => write!(f, "unsupported or corrupt file: {s}"),
            Self::NoAudioTrack => write!(f, "no audio track"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<std::io::Error> for DecodeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Decodes the whole file. `cancel` is polled between packets; `progress`
/// receives 0..1 when the track length is known.
pub fn decode_file(
    path: &Path,
    cancel: Option<&AtomicBool>,
    progress: &mut dyn FnMut(f32),
) -> Result<DecodedAudio, DecodeError> {
    let file = File::open(path)?;
    let ext = path.extension().and_then(|e| e.to_str());
    decode_source(Box::new(file), ext, cancel, progress, &mut |_| {})
}

/// Decodes from `reader` (`ext` is the file type's extension), such as a
/// file that is still downloading: reads may block until their bytes are
/// there. `on_frames` sees the audio decoded so far after every packet.
pub fn decode_reader<R: Read + Seek + Send + Sync + 'static>(
    reader: R,
    ext: Option<&str>,
    cancel: Option<&AtomicBool>,
    on_frames: &mut dyn FnMut(&DecodedAudio),
) -> Result<DecodedAudio, DecodeError> {
    decode_source(Box::new(Streaming(reader)), ext, cancel, &mut |_| {}, on_frames)
}

/// A seekable reader whose length is not told: the probe would otherwise
/// read the end of the file first, for tags, and wait for all of it to
/// download.
struct Streaming<R>(R);

impl<R: Read> Read for Streaming<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<R: Seek> Seek for Streaming<R> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl<R: Read + Seek + Send + Sync> MediaSource for Streaming<R> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

fn decode_source(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    cancel: Option<&AtomicBool>,
    progress: &mut dyn FnMut(f32),
    on_frames: &mut dyn FnMut(&DecodedAudio),
) -> Result<DecodedAudio, DecodeError> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| DecodeError::Unsupported(e.to_string()))?;
    let track = format.default_track(TrackType::Audio).ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;
    let total_frames = track.num_frames;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or(DecodeError::NoAudioTrack)?.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| DecodeError::Unsupported(e.to_string()))?;

    let mut out = DecodedAudio {
        sample_rate: params.sample_rate.unwrap_or(0),
        frames: Vec::with_capacity(total_frames.unwrap_or(0) as usize),
    };
    let mut scratch: Vec<f32> = Vec::new();
    let mut last_progress = 0.0f32;

    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(DecodeError::Cancelled);
        }
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => {
                // Truncated or damaged tail: keep what we have if it's substantial.
                if out.frames.is_empty() {
                    return Err(DecodeError::Unsupported(e.to_string()));
                }
                break;
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            // A corrupt packet is skipped, like every player does.
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(DecodeError::Unsupported(e.to_string())),
        };
        let channels = buf.spec().channels().count().max(1);
        if out.sample_rate == 0 {
            out.sample_rate = buf.spec().rate();
        }
        buf.copy_to_vec_interleaved(&mut scratch);
        out.frames.extend(scratch.chunks_exact(channels).map(|s| match channels {
            1 => [s[0], s[0]],
            _ => [s[0], s[1]],
        }));
        on_frames(&out);

        if let Some(total) = total_frames.filter(|&t| t > 0) {
            let p = (out.frames.len() as f64 / total as f64).min(1.0) as f32;
            if p - last_progress >= 0.01 {
                last_progress = p;
                progress(p);
            }
        }
    }

    if out.sample_rate == 0 {
        return Err(DecodeError::Unsupported("unknown sample rate".into()));
    }
    progress(1.0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Minimal 16-bit PCM WAV writer for fixtures.
    fn write_wav(path: &Path, sr: u32, channels: u16, samples: &[i16]) {
        let mut f = File::create(path).unwrap();
        let data_len = (samples.len() * 2) as u32;
        let byte_rate = sr * u32::from(channels) * 2;
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&channels.to_le_bytes()).unwrap();
        f.write_all(&sr.to_le_bytes()).unwrap();
        f.write_all(&byte_rate.to_le_bytes()).unwrap();
        f.write_all(&(channels * 2).to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_len.to_le_bytes()).unwrap();
        for s in samples {
            f.write_all(&s.to_le_bytes()).unwrap();
        }
    }

    #[test]
    fn decodes_wav_sample_exact() {
        let dir = std::env::temp_dir().join(format!("rille-decode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("impulses.wav");
        // Stereo, impulses at known frames.
        let mut s = vec![0i16; 48_000 * 2];
        for &frame in &[0usize, 1234, 47_999] {
            s[frame * 2] = 16384;
            s[frame * 2 + 1] = -16384;
        }
        write_wav(&path, 48_000, 2, &s);
        let a = decode_file(&path, None, &mut |_| {}).unwrap();
        assert_eq!(a.sample_rate, 48_000);
        assert_eq!(a.frames.len(), 48_000);
        for &frame in &[0usize, 1234, 47_999] {
            assert!((a.frames[frame][0] - 0.5).abs() < 1e-4);
            assert!((a.frames[frame][1] + 0.5).abs() < 1e-4);
        }
        assert_eq!(a.frames[1233], [0.0, 0.0]);

        let mono = dir.join("mono.wav");
        write_wav(&mono, 44_100, 1, &[8192, 0, -8192]);
        let m = decode_file(&mono, None, &mut |_| {}).unwrap();
        assert_eq!(m.frames.len(), 3);
        assert_eq!(m.frames[0][0], m.frames[0][1]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn decodes_from_a_reader_and_reports_progress() {
        let dir = std::env::temp_dir().join(format!("rille-decode-reader-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        write_wav(&path, 44_100, 2, &vec![1000i16; 44_100 * 2]);
        let bytes = std::fs::read(&path).unwrap();
        let mut seen = Vec::new();
        let a = decode_reader(std::io::Cursor::new(bytes), Some("wav"), None, &mut |so_far| {
            seen.push(so_far.frames.len());
        })
        .unwrap();
        assert_eq!(a.frames.len(), 44_100);
        assert!(seen.len() > 1 && seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&44_100));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_and_garbage_files_error() {
        assert!(decode_file(Path::new("/nonexistent.mp3"), None, &mut |_| {}).is_err());
        let p = std::env::temp_dir().join(format!("rille-garbage-{}.mp3", std::process::id()));
        std::fs::write(&p, b"not audio at all").unwrap();
        assert!(decode_file(&p, None, &mut |_| {}).is_err());
        std::fs::remove_file(p).ok();
    }
}
