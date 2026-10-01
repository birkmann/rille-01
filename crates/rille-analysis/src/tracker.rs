//! Dynamic-programming beat tracker (Ellis 2007): finds the beat sequence that
//! best follows onsets while keeping inter-beat intervals near the period.

/// Returns beat positions in (integer) frames.
pub fn track(odf: &[f32], period: f64, tightness: f64) -> Vec<usize> {
    let n = odf.len();
    if n == 0 || period < 2.0 {
        return Vec::new();
    }
    let min_back = (period / 2.0).round() as usize;
    let max_back = (period * 2.0).round() as usize;
    let penalty: Vec<f64> = (0..=max_back)
        .map(|d| if d < min_back { f64::NEG_INFINITY } else { -tightness * (d as f64 / period).ln().powi(2) })
        .collect();

    let mut score = vec![0.0f64; n];
    let mut back = vec![usize::MAX; n];
    for t in 0..n {
        let mut best = 0.0;
        let mut arg = usize::MAX;
        if t >= min_back {
            for d in min_back..=max_back.min(t) {
                let s = score[t - d] + penalty[d];
                if s > best {
                    best = s;
                    arg = t - d;
                }
            }
        }
        score[t] = f64::from(odf[t]) + best;
        back[t] = arg;
    }

    // Start from the best-scoring frame within the last period.
    let tail = n.saturating_sub(period.ceil() as usize);
    let mut t = (tail..n).max_by(|&a, &b| score[a].total_cmp(&score[b])).unwrap_or(n - 1);
    let mut beats = vec![t];
    while back[t] != usize::MAX {
        t = back[t];
        beats.push(t);
    }
    beats.reverse();
    beats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_regular_pulses() {
        let mut odf = vec![0.0f32; 2000];
        for i in (37..2000).step_by(80) {
            odf[i] = 5.0;
        }
        let beats = track(&odf, 80.0, 400.0);
        assert!(beats.len() >= 24);
        assert!(beats.iter().all(|b| (b - 37) % 80 == 0));
    }
}
