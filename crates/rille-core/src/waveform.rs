//! Waveform summary: per-bin peaks of three frequency bands, used for the
//! scrolling waveform and the overview.

/// Bins per second of audio.
pub const WAVEFORM_BINS_PER_SEC: f64 = 150.0;

const MAGIC: &[u8; 4] = b"TLWF";
const VERSION: u16 = 1;

/// Each bin is `[low, mid, high, full]`, peak amplitude with a square-root
/// curve applied (`round(255 * sqrt(peak))`), so quiet parts stay visible.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaveformSummary {
    pub bins_per_sec: f64,
    pub bins: Vec<[u8; 4]>,
}

impl WaveformSummary {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(14 + self.bins.len() * 4);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.bins_per_sec as f32).to_le_bytes());
        out.extend_from_slice(&(self.bins.len() as u32).to_le_bytes());
        for b in &self.bins {
            out.extend_from_slice(b);
        }
        out
    }

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 14 || &data[..4] != MAGIC || u16::from_le_bytes([data[4], data[5]]) != VERSION {
            return None;
        }
        let bins_per_sec = f64::from(f32::from_le_bytes(data[6..10].try_into().ok()?));
        let n = u32::from_le_bytes(data[10..14].try_into().ok()?) as usize;
        let body = data.get(14..14 + n * 4)?;
        let bins = body.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        Some(Self { bins_per_sec, bins })
    }

    /// Reduces to `n` bins (max per group), e.g. for the overview stripe.
    pub fn downsample(&self, n: usize) -> Vec<[u8; 4]> {
        if self.bins.is_empty() || n == 0 {
            return Vec::new();
        }
        (0..n)
            .map(|i| {
                let a = i * self.bins.len() / n;
                let b = ((i + 1) * self.bins.len() / n).max(a + 1).min(self.bins.len());
                self.bins[a..b].iter().fold([0u8; 4], |m, v| std::array::from_fn(|k| m[k].max(v[k])))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_downsample() {
        let w = WaveformSummary { bins_per_sec: 150.0, bins: (0..100u8).map(|i| [i, i / 2, 3, i]).collect() };
        assert_eq!(WaveformSummary::from_bytes(&w.to_bytes()), Some(w.clone()));
        assert!(WaveformSummary::from_bytes(b"junk").is_none());
        let d = w.downsample(10);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], [9, 4, 3, 9]);
    }
}
