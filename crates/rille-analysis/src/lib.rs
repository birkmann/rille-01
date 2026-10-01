//! Track analysis: beatgrid, key, loudness and waveform summary.
//!
//! The beatgrid is the priority: see [`grid`] for the pipeline and
//! `tests/synthetic.rs` for the accuracy targets it is held to.

pub mod downbeat;
pub mod filter;
pub mod fit;
pub mod grid;
pub mod key;
pub mod kick;
pub mod loudness;
pub mod onset;
pub mod phase;
pub mod quality;
pub mod refine;
pub mod synth;
pub mod tempo;
pub mod tracker;
pub mod waveform;

use std::path::Path;
use std::sync::atomic::AtomicBool;

use filter::{Biquad, filtfilt};
use rille_core::track::{ANALYZER_VERSION, TrackAnalysis};
use rille_core::waveform::WaveformSummary;
use rille_decode::{DecodeError, DecodedAudio};

pub use grid::GridReport;

#[derive(Clone, Debug)]
pub struct AnalysisConfig {
    /// Detected tempos are folded into this range (half/double).
    pub bpm_range: (f64, f64),
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self { bpm_range: (88.0, 175.0) }
    }
}

pub struct AnalysisOutput {
    pub analysis: TrackAnalysis,
    pub waveform: WaveformSummary,
    pub report: GridReport,
}

pub fn analyze(audio: &DecodedAudio, cfg: &AnalysisConfig) -> AnalysisOutput {
    let sr = f64::from(audio.sample_rate);
    let mono = audio.mono();
    std::thread::scope(|s| {
        let wave = s.spawn(|| waveform::build(&audio.frames, sr));
        let loud = s.spawn(|| {
            (
                loudness::integrated_lufs(&audio.frames, sr),
                loudness::peak_db(&audio.frames),
                loudness::first_sound_secs(&audio.frames, sr),
            )
        });

        let dec = decimated(&mono, sr);
        let key = key::detect(&dec.chroma);
        let (grid, report) = grid::detect(&mono, sr, &dec, cfg.bpm_range);
        let (lufs, peak_db, first_sound_secs) = loud.join().expect("loudness thread");
        let analysis = TrackAnalysis {
            analyzer_version: ANALYZER_VERSION,
            duration_secs: audio.duration_secs(),
            sample_rate: audio.sample_rate,
            grid,
            key: key.map(|k| k.0),
            key_confidence: key.map_or(0.0, |k| k.1),
            lufs,
            peak_db,
            first_sound_secs,
        };
        AnalysisOutput { analysis, waveform: wave.join().expect("waveform thread"), report }
    })
}

/// Decodes and analyzes a file. Returns the decoded audio too, so a deck
/// load can reuse it.
pub fn analyze_file(
    path: &Path,
    cfg: &AnalysisConfig,
    cancel: Option<&AtomicBool>,
) -> Result<(DecodedAudio, AnalysisOutput), DecodeError> {
    let audio = rille_decode::decode_file(path, cancel, &mut |_| {})?;
    let out = analyze(&audio, cfg);
    Ok((audio, out))
}

/// Mono signal decimated to ~11 kHz for key and downbeat analysis.
pub fn decimate_for_key(mono: &[f32], sr: f64) -> (Vec<f32>, f64) {
    let factor = (sr / 11_025.0).round().max(1.0) as usize;
    (filter::decimate(mono, sr, factor), sr / factor as f64)
}

fn decimated(mono: &[f32], sr: f64) -> grid::Decimated {
    let factor = (sr / 11_025.0).round().max(1.0) as usize;
    let x = filter::decimate(mono, sr, factor);
    let dsr = sr / factor as f64;
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let mut low = x.clone();
    filtfilt(&[Biquad::lowpass(dsr, 150.0, q)], &mut low);
    let mut mid = x.clone();
    filtfilt(&[Biquad::highpass(dsr, 150.0, q), Biquad::lowpass(dsr, 2000.0, q)], &mut mid);
    let mut high = x.clone();
    filtfilt(&[Biquad::highpass(dsr, 2000.0, q)], &mut high);
    let chroma = key::chroma(&x, dsr);
    grid::Decimated { sr: dsr, bands: [low, mid, high], chroma }
}
