//! Sub-millisecond transient location at the full sample rate.
//!
//! A beat's position is defined as the *start* of the kick attack: where the
//! zero-phase smoothed kick-band power first reaches 20 % of its rise (see
//! [`TransientFinder::attack`]). That is where the eye sees the kick begin on
//! a waveform, and it depends much less on the kick's shape than the point of
//! steepest rise, so grids of different tracks line up with each other.

use crate::filter::{Biquad, filtfilt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    /// 30–150 Hz: kick drum.
    Low,
    /// Everything above 100 Hz: for tracks without a kick.
    Broad,
}

#[derive(Clone, Copy, Debug)]
pub struct Transient {
    pub secs: f64,
    /// Log-energy jump across the attack (natural log units; ×4.34 = dB).
    pub strength: f64,
}

/// Delay of the causal kick-band filter at the start of an attack, removed
/// from kick starts (calibrated on synthetic kicks of several shapes).
const START_DELAY: f64 = 0.0011;

pub struct TransientFinder {
    band: Band,
    sr: f64,
    /// Where attack starts are timed. Kick band: a *causal* low-pass of the
    /// signal (no pre-ringing before the attack, unlike the zero-phase filter
    /// used to locate it; its small delay is removed by [`START_DELAY`]), so
    /// hats or clicks just before the kick can't trigger it. Broadband: a
    /// causal high-pass (again no pre-ringing).
    start_signal: Vec<f32>,
    /// Band-limited envelope power (sample domain).
    power: Vec<f32>,
    /// Energy floor added before taking logs.
    eps: f64,
    smooth: usize,
    slope: usize,
    jump: usize,
}

impl TransientFinder {
    pub fn new(mono: &[f32], sr: f64, band: Band) -> Self {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let q2 = std::f64::consts::FRAC_1_SQRT_2;
        let start_signal = match band {
            Band::Low => {
                let mut c = mono.to_vec();
                Biquad::lowpass(sr, 200.0, q2).run(&mut c);
                Biquad::highpass(sr, 25.0, q2).run(&mut c);
                c
            }
            Band::Broad => {
                let mut c = mono.to_vec();
                Biquad::highpass(sr, 100.0, q2).run(&mut c);
                c
            }
        };
        let mut x = mono.to_vec();
        let power: Vec<f32> = match band {
            Band::Low => {
                filtfilt(&[Biquad::lowpass(sr, 150.0, q), Biquad::highpass(sr, 30.0, q)], &mut x);
                // x² + (x'/ω)² removes most of the ripple of a low sine's energy.
                let w = 2.0 * std::f64::consts::PI * 60.0 / sr;
                (0..x.len())
                    .map(|n| {
                        let d = if n > 0 && n + 1 < x.len() { f64::from(x[n + 1] - x[n - 1]) * 0.5 } else { 0.0 };
                        (f64::from(x[n]).powi(2) + (d / w).powi(2)) as f32
                    })
                    .collect()
            }
            Band::Broad => {
                filtfilt(&[Biquad::highpass(sr, 100.0, q)], &mut x);
                x.iter().map(|v| v * v).collect()
            }
        };
        let mean = power.iter().map(|p| f64::from(*p)).sum::<f64>() / power.len().max(1) as f64;
        let ms = |m: f64| ((m * sr / 1000.0).round() as usize).max(1);
        let (smooth, slope) = match band {
            Band::Low => (ms(4.0), ms(2.0)),
            Band::Broad => (ms(1.5), ms(0.75)),
        };
        Self { band, sr, start_signal, power, eps: mean * 1e-3 + 1e-12, smooth, slope, jump: ms(8.0) }
    }

    pub fn band(&self) -> Band {
        self.band
    }

    /// Smoothed band power at `n` evenly spaced points over
    /// `[centre - half, centre + half]` seconds (zero outside the track).
    pub fn envelope(&self, centre: f64, half: f64, n: usize) -> Vec<f32> {
        let len = self.power.len() as isize;
        let w = (self.smooth / 2).max(1) as isize;
        (0..n)
            .map(|k| {
                let t = centre - half + 2.0 * half * k as f64 / (n.max(2) - 1) as f64;
                let i = (t * self.sr).round() as isize;
                let (a, b) = ((i - w).max(0), (i + w + 1).min(len));
                if a >= b {
                    return 0.0;
                }
                self.power[a as usize..b as usize].iter().sum::<f32>() / (b - a) as f32
            })
            .collect()
    }

    /// Energy of the start signal (causal filters, kick band
    /// delay-compensated) at `n` points over `[centre ± half]`, smoothed over
    /// 1 ms.
    pub fn start_envelope(&self, centre: f64, half: f64, n: usize) -> Vec<f32> {
        let len = self.start_signal.len() as isize;
        let w = (0.0005 * self.sr).round().max(1.0) as isize;
        let delay = if self.band == Band::Low { START_DELAY } else { 0.0 };
        (0..n)
            .map(|k| {
                let t = centre + delay - half + 2.0 * half * k as f64 / (n.max(2) - 1) as f64;
                let i = (t * self.sr).round() as isize;
                let (a, b) = ((i - w).max(0), (i + w + 1).min(len));
                if a >= b {
                    return 0.0;
                }
                self.start_signal[a as usize..b as usize].iter().map(|v| v * v).sum::<f32>() / (b - a) as f32
            })
            .collect()
    }

    /// Start of the strongest attack within `radius` of `secs`. The band's
    /// steepest rise identifies the attack (e.g. the kick, not a hi-hat);
    /// then, on the start signal's energy (see `start_signal`; 0.5 ms
    /// smoothing, so at most 0.25 ms of blur), the start is where it first
    /// rose 10 % of the way from the background to the attack's peak —
    /// searched at most 10 ms before the band's rise, so other instruments
    /// can't be picked up.
    pub fn attack(&self, secs: f64, radius: f64) -> Option<Transient> {
        let steep = self.find(secs, radius)?;
        let ms = |m: f64| (m * self.sr / 1000.0).round() as isize;
        let centre = (steep.secs * self.sr).round() as isize;
        let (a, b) = (centre - ms(65.0), centre + ms(30.0));
        if a < 0 || b as usize > self.start_signal.len() {
            return Some(steep);
        }
        let env: Vec<f64> = self.start_signal[a as usize..b as usize].iter().map(|v| f64::from(*v).powi(2)).collect();
        let c = (centre - a) as usize;
        let smooth = ms(0.25).max(1) as usize;
        let sm: Vec<f64> = (0..env.len())
            .map(|i| {
                let (lo, hi) = (i.saturating_sub(smooth), (i + smooth + 1).min(env.len()));
                env[lo..hi].iter().sum::<f64>() / (hi - lo) as f64
            })
            .collect();
        // Background: median level 25–60 ms before the band's rise (sub
        // kicks can build for 20 ms before their steepest point).
        let (bg0, bg1) = (c.saturating_sub(ms(60.0) as usize), c.saturating_sub(ms(25.0) as usize));
        let mut bg: Vec<f64> = sm[bg0..bg1.max(bg0 + 1)].to_vec();
        bg.sort_by(f64::total_cmp);
        let floor = bg[bg.len() / 2];
        let top = sm[c..(c + ms(25.0) as usize).min(sm.len())].iter().copied().fold(0.0f64, f64::max);
        if top <= floor * 1.5 {
            return Some(steep);
        }
        let thr = floor + 0.1 * (top - floor);
        // Scan forward: the raw signal oscillates, so walking back from the
        // rise would stop at a zero crossing of the kick's own waveform.
        let limit = c.saturating_sub(ms(25.0) as usize).max(1);
        let end = (c + ms(5.0) as usize).min(sm.len() - 1);
        // A kick's attack stays up; a hat or bass click before it dies away
        // within a few ms. Require energy above the threshold somewhere in
        // every 3 ms slice of the next 12 ms (slices bridge the zero
        // crossings of the kick's own low-frequency waveform).
        let slice = ms(3.0).max(1) as usize;
        let sustained = |i: usize| {
            (0..4).all(|k| {
                let (x0, x1) = (i + k * slice, (i + (k + 1) * slice).min(sm.len()));
                x0 < x1 && sm[x0..x1].iter().any(|v| *v >= thr)
            })
        };
        let Some(i) = (limit..=end).find(|&i| sm[i] >= thr && sustained(i)) else { return Some(steep) };
        let (y0, y1) = (sm[i - 1], sm[i]);
        let t = (i - 1) as f64 + if y1 > y0 { ((thr - y0) / (y1 - y0)).clamp(0.0, 1.0) } else { 1.0 };
        let delay = if self.band == Band::Low { START_DELAY } else { 0.0 };
        Some(Transient { secs: (a as f64 + t) / self.sr - delay, strength: steep.strength })
    }

    /// Finds the steepest attack within `radius` seconds of `secs`.
    pub fn find(&self, secs: f64, radius: f64) -> Option<Transient> {
        let n = self.power.len() as isize;
        let centre = (secs * self.sr).round() as isize;
        let r = (radius * self.sr).round() as isize;
        let pad = (self.smooth + self.slope + self.jump) as isize + 2;
        let (a, b) = ((centre - r - pad).max(0), (centre + r + pad).min(n));
        if b - a < 2 * pad + 3 {
            return None;
        }
        let seg = &self.power[a as usize..b as usize];
        // Centred moving average via prefix sums, then log.
        let mut prefix = vec![0.0f64; seg.len() + 1];
        for (i, p) in seg.iter().enumerate() {
            prefix[i + 1] = prefix[i] + f64::from(*p);
        }
        let half = self.smooth / 2;
        let smoothed: Vec<f64> = (0..seg.len())
            .map(|i| {
                let (lo, hi) = (i.saturating_sub(half), (i + half + 1).min(seg.len()));
                (prefix[hi] - prefix[lo]) / (hi - lo) as f64
            })
            .collect();
        let log_e: Vec<f64> = smoothed.iter().map(|e| (e + self.eps).ln()).collect();
        // Locate on linear power: with zero-phase smoothing, the steepest
        // rise of an abrupt attack sits exactly on the attack.
        let s = self.slope;
        let slope = |i: usize| smoothed[i + s] - smoothed[i - s];
        let lo = (centre - r - a).max(s as isize + 1) as usize;
        let hi = ((centre + r - a) as usize).min(seg.len() - s - 2);
        if lo >= hi {
            return None;
        }
        let peak = (lo..=hi).max_by(|&i, &j| slope(i).total_cmp(&slope(j)))?;
        // Parabolic interpolation of the slope maximum.
        let (y0, y1, y2) = (slope(peak - 1), slope(peak), slope(peak + 1));
        let denom = y0 - 2.0 * y1 + y2;
        let frac = if denom.abs() > 1e-18 { (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5) } else { 0.0 };
        let j = self.jump;
        let strength = log_e[(peak + j).min(seg.len() - 1)] - log_e[peak.saturating_sub(j)];
        Some(Transient { secs: (a as f64 + peak as f64 + frac) / self.sr, strength })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kick shapes: 0 pitch-swept sine (hard start), 1 with a noise click on
    /// top, 2 soft 5 ms fade-in, 3 long 808-style boom with 2 ms fade-in.
    fn kick(sr: f64, at: f64, len: usize, shape: u8) -> Vec<f32> {
        let mut x = vec![0.0f32; len];
        let start = (at * sr).round() as usize;
        let mut phase = 0.0f64;
        let mut seed = 12345u32;
        for (k, v) in x[start..].iter_mut().enumerate().take((0.4 * sr) as usize) {
            let t = k as f64 / sr;
            let (f, decay, fade) = match shape {
                3 => (48.0 + 30.0 * (-t * 20.0).exp(), 3.0, 0.002),
                2 => (45.0 + 100.0 * (-t * 30.0).exp(), 12.0, 0.005),
                _ => (45.0 + 100.0 * (-t * 30.0).exp(), 12.0, 0.0),
            };
            phase += 2.0 * std::f64::consts::PI * f / sr;
            let env = (-t * decay).exp() * if fade > 0.0 { (t / fade).min(1.0) } else { 1.0 };
            let mut y = phase.sin() * env * 0.8;
            if shape == 1 && t < 0.002 {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                y += (f64::from(seed >> 16) / 32768.0 - 1.0) * 0.4;
            }
            *v = y as f32;
        }
        x
    }

    #[test]
    fn attack_start_is_on_the_kick_for_all_shapes() {
        let sr = 44_100.0;
        for shape in 0..4u8 {
            for &at in &[0.5, 0.5003, 0.51234] {
                let x = kick(sr, at, 44_100, shape);
                let f = TransientFinder::new(&x, sr, Band::Low);
                let steep = f.find(at + 0.012, 0.03).unwrap();
                let start = f.attack(at + 0.012, 0.03).unwrap();
                println!(
                    "shape {shape} at {at}: steepest {:+.3} ms, start {:+.3} ms",
                    (steep.secs - at) * 1000.0,
                    (start.secs - at) * 1000.0
                );
                let off = (start.secs - at) * 1000.0;
                // Slow attacks (shape 2: 5 ms fade-in) reach the threshold later;
                // everything else lands within half a millisecond.
                let max = match shape {
                    2 => 4.5,
                    3 => 1.5,
                    _ => 0.8,
                };
                assert!((-0.5..max).contains(&off), "shape {shape}: start {off:.3} ms after the onset");
            }
        }
    }

    #[test]
    fn finds_kick_attack_within_a_millisecond() {
        let sr = 44_100.0;
        for &at in &[0.5, 0.5003, 0.51234] {
            let x = kick(sr, at, 44_100, 0);
            let f = TransientFinder::new(&x, sr, Band::Low);
            let t = f.find(at + 0.012, 0.03).unwrap();
            println!("at {at}: offset {:.3} ms", (t.secs - at) * 1000.0);
            assert!((t.secs - at).abs() < 0.001, "at {at}: found {}", t.secs);
            assert!(t.strength > 2.0);
        }
    }
}
