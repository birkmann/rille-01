//! Stem separation: drums, bass, other and vocals of a track, with Meta's
//! HTDemucs model (the ONNX export with the spectrogram inside the graph),
//! run on the CPU with tract.
//!
//! The model's weights are not part of rille: Meta released them for
//! research use only, so the app downloads them when the user asks (see
//! [`MODEL_URL`]).
//!
//! The track is resampled to the model's 44.1 kHz, normalized as Demucs
//! does, cut into overlapping segments of 7.8 s that are separated in
//! parallel and blended back with triangular weights, and resampled to the
//! track's rate. Only drums, bass and vocals are kept: the other stem is
//! the track minus those three, so the stems add up to the track exactly.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rille_dsp::SincTable;
use tract_onnx::prelude::*;

/// Where the model is downloaded from: a pinned revision of an ONNX export
/// of HTDemucs v4 (Meta's `htdemucs`, 4 stems).
pub const MODEL_URL: &str =
    "https://huggingface.co/StemSplitio/htdemucs-onnx/resolve/d54ed9eb60e258ea82131c6ee14578628816456a/htdemucs.onnx";
/// SHA-256 of the model file.
pub const MODEL_SHA256: &str = "68d0bf16428ef66e692cdff8a9ccf28f1ef3f69440d57e58605a4cc55fcc5e74";
pub const MODEL_BYTES: u64 = 316_446_953;
/// Shown before the download.
pub const MODEL_NOTICE: &str = "Stem separation uses Meta's HTDemucs model. Its weights were released for research use \
     only and are not part of rille; they are downloaded from Hugging Face (316 MB).";

/// The model's sample rate.
pub const MODEL_RATE: u32 = 44_100;
/// Samples per segment (7.8 s at 44.1 kHz).
const SEGMENT: usize = 343_980;
/// Segments overlap by a quarter.
const STRIDE: usize = SEGMENT * 3 / 4;
/// Model outputs, in order.
const SOURCES: usize = 4;
/// The model's output order, and the stems kept (drums, bass, vocals).
const KEPT: [usize; 3] = [0, 1, 3];

/// Drums, bass and vocals as 16-bit stereo per frame (drums L R, bass L R,
/// vocals L R), at the input's sample rate and length.
pub type Stems = Vec<[i16; 6]>;

/// A loaded model. Loading takes a while (half a minute on a fast CPU), so
/// keep it for the next tracks.
pub struct Separator {
    plan: Arc<TypedRunnableModel>,
}

impl Separator {
    pub fn load(path: &Path) -> Result<Self, String> {
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(|e| format!("cannot load the stem model: {e}"))?;
        Ok(Self { plan: model })
    }

    /// Separates `frames` (stereo at `sample_rate`), running `threads`
    /// segments at a time. `progress` gets 0..1; `cancel` stops between
    /// segments.
    pub fn separate(
        &self,
        frames: &[[f32; 2]],
        sample_rate: u32,
        threads: usize,
        cancel: &AtomicBool,
        progress: &(dyn Fn(f32) + Sync),
    ) -> Result<Stems, String> {
        let run = |segment: &[f32]| -> Result<Vec<f32>, String> {
            let input =
                tract_ndarray::Array3::from_shape_vec((1, 2, SEGMENT), segment.to_vec()).map_err(|e| e.to_string())?;
            let out = self.plan.run(tvec!(Tensor::from(input).into())).map_err(|e| e.to_string())?;
            let view = out[0].to_plain_array_view::<f32>().map_err(|e| e.to_string())?;
            if view.len() != SOURCES * 2 * SEGMENT {
                return Err(format!("unexpected model output shape {:?}", view.shape()));
            }
            Ok(view.iter().copied().collect())
        };
        separate_with(frames, sample_rate, threads, cancel, progress, &run)
    }
}

/// The model as a function, see [`separate_with`].
type RunSegment<'a> = dyn Fn(&[f32]) -> Result<Vec<f32>, String> + Sync + 'a;

/// [`Separator::separate`] with the model as a function: a planar stereo
/// segment (`2 × SEGMENT`) in, the four sources (`4 × 2 × SEGMENT`) out.
fn separate_with(
    frames: &[[f32; 2]],
    sample_rate: u32,
    threads: usize,
    cancel: &AtomicBool,
    progress: &(dyn Fn(f32) + Sync),
    run: &RunSegment<'_>,
) -> Result<Stems, String> {
    if frames.is_empty() {
        return Ok(Vec::new());
    }
    let x = resample(frames, sample_rate, MODEL_RATE);
    let n = x.len();
    // Normalized as Demucs does: by the mono mix's mean and deviation.
    let mono: Vec<f64> = x.iter().map(|f| f64::from(f[0] + f[1]) / 2.0).collect();
    let mean = mono.iter().sum::<f64>() / n as f64;
    let var = mono.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n.max(2) - 1) as f64;
    let (mean, std) = (mean as f32, (var.sqrt() + 1e-8) as f32);

    let weight: Vec<f32> = (0..SEGMENT).map(|i| (i + 1).min(SEGMENT - i) as f32).collect();
    let peak = weight.iter().copied().fold(0.0, f32::max);
    let weight: Vec<f32> = weight.iter().map(|w| w / peak).collect();
    let offsets: Vec<usize> = (0..n).step_by(STRIDE).collect();

    // Sources (only the kept ones) and summed weights, per model-rate frame.
    let acc = Mutex::new((vec![[0.0f32; 6]; n], vec![0.0f32; n]));
    let (next, done) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let failed: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, offsets.len()) {
            s.spawn(|| {
                let mut segment = vec![0.0f32; 2 * SEGMENT];
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&start) = offsets.get(k) else { break };
                    if cancel.load(Ordering::Relaxed) || failed.lock().expect("failed lock").is_some() {
                        break;
                    }
                    let len = SEGMENT.min(n - start);
                    segment.fill(0.0);
                    for (i, f) in x[start..start + len].iter().enumerate() {
                        segment[i] = (f[0] - mean) / std;
                        segment[SEGMENT + i] = (f[1] - mean) / std;
                    }
                    let out = match run(&segment) {
                        Ok(out) => out,
                        Err(e) => {
                            failed.lock().expect("failed lock").get_or_insert(e);
                            break;
                        }
                    };
                    let mut acc = acc.lock().expect("stems lock");
                    let (sum, wsum) = &mut *acc;
                    for i in 0..len {
                        let w = weight[i];
                        for (j, &src) in KEPT.iter().enumerate() {
                            for ch in 0..2 {
                                sum[start + i][2 * j + ch] += w * out[(src * 2 + ch) * SEGMENT + i];
                            }
                        }
                        wsum[start + i] += w;
                    }
                    drop(acc);
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    progress(d as f32 / offsets.len() as f32);
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().expect("failed lock") {
        return Err(e);
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let (sum, wsum) = acc.into_inner().expect("stems lock");
    // Undo the normalization; the mean belongs to the whole mix, which the
    // other stem (the rest) keeps.
    let stems: Vec<[f32; 6]> = sum.iter().zip(&wsum).map(|(s, &w)| s.map(|v| v / w.max(1e-9) * std)).collect();
    // Back to the track's rate, one stereo stem at a time.
    let back: Vec<Vec<[f32; 2]>> = (0..3)
        .map(|j| {
            let stem: Vec<[f32; 2]> = stems.iter().map(|f| [f[2 * j], f[2 * j + 1]]).collect();
            let mut out = resample(&stem, MODEL_RATE, sample_rate);
            out.resize(frames.len(), [0.0; 2]);
            out
        })
        .collect();
    let q = |v: f32| (v * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
    Ok((0..frames.len())
        .map(|i| {
            let [d, b, v] = [back[0][i], back[1][i], back[2][i]];
            [q(d[0]), q(d[1]), q(b[0]), q(b[1]), q(v[0]), q(v[1])]
        })
        .collect())
}

/// `frames` at `from` Hz resampled to `to` Hz with a windowed-sinc kernel.
fn resample(frames: &[[f32; 2]], from: u32, to: u32) -> Vec<[f32; 2]> {
    if from == to || from == 0 || to == 0 {
        return frames.to_vec();
    }
    let sinc = SincTable::new(32, 512);
    let ratio = f64::from(from) / f64::from(to);
    let cutoff = SincTable::cutoff_for_ratio(ratio);
    let len = (frames.len() as f64 / ratio).round() as usize;
    (0..len).map(|i| sinc.sample(frames, i as f64 * ratio, cutoff)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A "model" that splits a segment by a fixed share per source, so the
    /// blending and normalization can be checked exactly.
    fn shares(segment: &[f32]) -> Result<Vec<f32>, String> {
        let share = [0.5, 0.25, 0.15, 0.1];
        Ok(share.iter().flat_map(|s| segment.iter().map(move |v| v * s)).collect())
    }

    #[test]
    fn segments_blend_back_into_the_track() {
        let n = 3 * SEGMENT + 12_345;
        let frames: Vec<[f32; 2]> =
            (0..n).map(|i| [((i as f32) * 0.003).sin() * 0.5 + 0.05, ((i as f32) * 0.0017).cos() * 0.3]).collect();
        let calls = AtomicUsize::new(0);
        let run = |s: &[f32]| {
            calls.fetch_add(1, Ordering::Relaxed);
            shares(s)
        };
        let last = Mutex::new(0.0f32);
        let progress = |p: f32| *last.lock().unwrap() = p;
        let stems = separate_with(&frames, MODEL_RATE, 3, &AtomicBool::new(false), &progress, &run).unwrap();
        assert_eq!(stems.len(), n);
        assert_eq!(calls.load(Ordering::Relaxed), n.div_ceil(STRIDE));
        assert_eq!(*last.lock().unwrap(), 1.0);
        // Each kept stem is its share of the track without its mean (the
        // rest keeps that), to 16-bit precision.
        let mean = frames.iter().map(|f| f64::from(f[0] + f[1]) / 2.0).sum::<f64>() as f32 / n as f32;
        for i in (0..n).step_by(9_999) {
            for (j, share) in [0.5f32, 0.25, 0.1].iter().enumerate() {
                for ch in 0..2 {
                    let want = (frames[i][ch] - mean) * share;
                    let got = f32::from(stems[i][2 * j + ch]) / 32768.0;
                    assert!((got - want).abs() < 1e-4, "frame {i} stem {j}: {got} vs {want}");
                }
            }
        }
    }

    #[test]
    fn other_rates_are_resampled_both_ways() {
        let frames: Vec<[f32; 2]> = (0..96_000).map(|i| [((i as f32) * 0.01).sin() * 0.5; 2]).collect();
        let stems = separate_with(&frames, 48_000, 2, &AtomicBool::new(false), &|_| {}, &shares).unwrap();
        assert_eq!(stems.len(), frames.len());
        let mid = 50_000;
        let got = f32::from(stems[mid][0]) / 32768.0;
        let mean = frames.iter().map(|f| f[0]).sum::<f32>() / frames.len() as f32;
        assert!((got - (frames[mid][0] - mean) * 0.5).abs() < 2e-3, "{got}");
    }

    #[test]
    fn errors_and_cancelling_stop_the_separation() {
        let frames = vec![[0.1f32; 2]; SEGMENT * 2];
        let fail = |_: &[f32]| -> Result<Vec<f32>, String> { Err("broken".into()) };
        let err = separate_with(&frames, MODEL_RATE, 2, &AtomicBool::new(false), &|_| {}, &fail);
        assert_eq!(err, Err("broken".into()));
        let cancelled = separate_with(&frames, MODEL_RATE, 2, &AtomicBool::new(true), &|_| {}, &shares);
        assert_eq!(cancelled, Err("cancelled".into()));
    }

    /// With `RILLE_STEM_MODEL=<htdemucs.onnx>`: the real model on a mix of a
    /// kick and a tone puts the kick in the drums.
    #[test]
    #[ignore]
    fn real_model_separates() {
        let Some(path) = std::env::var_os("RILLE_STEM_MODEL") else { return };
        let t = std::time::Instant::now();
        let sep = Separator::load(Path::new(&path)).unwrap();
        eprintln!("loaded in {:?}", t.elapsed());
        let sr = MODEL_RATE as usize;
        let frames: Vec<[f32; 2]> = (0..10 * sr)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let beat = t % 0.5;
                let kick = (-beat * 30.0).exp() * (2.0 * std::f32::consts::PI * 55.0 * beat).sin() * 0.6;
                let tone = (2.0 * std::f32::consts::PI * 660.0 * t).sin() * 0.2;
                [kick + tone; 2]
            })
            .collect();
        let t = std::time::Instant::now();
        let stems = sep.separate(&frames, MODEL_RATE, 4, &AtomicBool::new(false), &|_| {}).unwrap();
        eprintln!("separated 10 s in {:?}", t.elapsed());
        let rms = |j: usize| {
            (stems.iter().map(|f| (f32::from(f[2 * j]) / 32768.0).powi(2)).sum::<f32>() / stems.len() as f32).sqrt()
        };
        let (drums, bass, vocals) = (rms(0), rms(1), rms(2));
        eprintln!("rms drums {drums:.4}, bass {bass:.4}, vocals {vocals:.4}");
        assert!(drums > vocals);
    }
}
