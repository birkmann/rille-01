//! Integrated loudness (ITU-R BS.1770 / EBU R128 gating) and sample peak.

use crate::filter::Biquad;

pub fn integrated_lufs(frames: &[[f32; 2]], sr: f64) -> Option<f32> {
    // K-weighting: high shelf + high-pass (BS.1770 constants).
    let shelf = Biquad::high_shelf(sr, 1681.974, 0.7071752, 3.99984);
    let hp = Biquad::highpass(sr, 38.13547, 0.5003270);
    let mut ch: [Vec<f32>; 2] = [frames.iter().map(|f| f[0]).collect(), frames.iter().map(|f| f[1]).collect()];
    for c in &mut ch {
        shelf.run(c);
        hp.run(c);
    }
    let block = (0.4 * sr) as usize;
    let step = (0.1 * sr) as usize;
    if frames.len() < block {
        return None;
    }
    let mut blocks = Vec::new();
    let mut start = 0;
    while start + block <= frames.len() {
        let z: f64 = ch
            .iter()
            .map(|c| c[start..start + block].iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / block as f64)
            .sum();
        blocks.push(z);
        start += step;
    }
    let lufs = |z: f64| -0.691 + 10.0 * z.max(1e-20).log10();
    let abs: Vec<f64> = blocks.into_iter().filter(|&z| lufs(z) > -70.0).collect();
    if abs.is_empty() {
        return None;
    }
    let rel_gate = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let gated: Vec<f64> = abs.into_iter().filter(|&z| lufs(z) > rel_gate).collect();
    Some(lufs(gated.iter().sum::<f64>() / gated.len().max(1) as f64) as f32)
}

pub fn peak_db(frames: &[[f32; 2]]) -> Option<f32> {
    let peak = frames.iter().flat_map(|f| f.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
    (peak > 0.0).then(|| 20.0 * peak.log10())
}

/// First position louder than −50 dBFS.
pub fn first_sound_secs(frames: &[[f32; 2]], sr: f64) -> f64 {
    let thr = 10f32.powf(-50.0 / 20.0);
    frames.iter().position(|f| f[0].abs() > thr || f[1].abs() > thr).map_or(0.0, |i| i as f64 / sr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_1k_sine_is_about_minus_3_lufs() {
        let sr = 48_000.0;
        let frames: Vec<[f32; 2]> = (0..(sr as usize * 5))
            .map(|i| {
                let v = (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / sr).sin() as f32;
                [v, v]
            })
            .collect();
        // BS.1770: a 0 dBFS 1 kHz sine in both channels reads about +0.6 LUFS
        // above −3.01 per channel, i.e. ≈ −0.0x LUFS stereo sum.
        let l = integrated_lufs(&frames, sr).unwrap();
        assert!((l - 0.0).abs() < 0.3, "{l}");
        assert!((peak_db(&frames).unwrap()).abs() < 0.01);
    }
}
