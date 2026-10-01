//! Synthetic dance tracks with exactly known beat positions, for testing and
//! evaluating the beatgrid analysis.

use rille_core::Key;
use rille_decode::DecodedAudio;

#[derive(Clone, Debug)]
pub struct Spec {
    pub sr: u32,
    /// `(bars, bpm)` sections; more than one gives tempo changes.
    pub sections: Vec<(usize, f64)>,
    /// Silence before the first beat.
    pub first_beat_secs: f64,
    /// Random per-beat timing error (standard deviation), like a live drummer.
    pub jitter_ms: f64,
    /// Random-walk tempo drift per bar (relative standard deviation).
    pub drift_per_bar: f64,
    /// Bars `[from, to)` without kick drum (a breakdown).
    pub kickless_bars: Option<(usize, usize)>,
    pub key: Key,
    /// Techno-style rolling sub bass: a loud note on each of the three
    /// sixteenths after every kick, each swelling up out of the sidechain
    /// dip, instead of the single off-beat bass note.
    pub rolling_bass: bool,
    pub seed: u64,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            sr: 44_100,
            sections: vec![(64, 128.0)],
            first_beat_secs: 0.5,
            jitter_ms: 0.0,
            drift_per_bar: 0.0,
            kickless_bars: None,
            key: Key::new(9, true),
            rolling_bass: false,
            seed: 1,
        }
    }
}

pub struct Rendered {
    pub audio: DecodedAudio,
    /// Onset time of every beat; `beats[0]` and every fourth beat are downbeats.
    pub beats: Vec<f64>,
    /// The underlying tempo pulse without per-beat jitter.
    pub pulse: Vec<f64>,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn noise(&mut self) -> f32 {
        (self.next() * 2.0 - 1.0) as f32
    }

    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.next().max(1e-12), self.next());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

fn midi_hz(n: f64) -> f64 {
    440.0 * 2f64.powf((n - 69.0) / 12.0)
}

pub fn render(spec: &Spec) -> Rendered {
    let sr = f64::from(spec.sr);
    let mut rng = Rng(spec.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);

    // Beat times.
    let mut pulse = Vec::new();
    let mut t = spec.first_beat_secs;
    let mut drift = 1.0;
    for &(bars, bpm) in &spec.sections {
        for _ in 0..bars {
            drift *= 1.0 + spec.drift_per_bar * rng.gauss();
            let beat = 60.0 / (bpm * drift);
            for _ in 0..4 {
                pulse.push(t);
                t += beat;
            }
        }
    }
    let beats: Vec<f64> = pulse.iter().map(|p| p + spec.jitter_ms / 1000.0 * rng.gauss()).collect();
    let len = ((t + 2.0) * sr) as usize;
    let mut x = vec![0.0f32; len];

    let tonic = f64::from(spec.key.pitch_class());
    // Chord roots (semitones above tonic) and whether each chord is minor:
    // i–iv–i–V (harmonic minor) or I–IV–I–V.
    let progression: [(f64, bool); 4] = if spec.key.is_minor() {
        [(0.0, true), (5.0, true), (0.0, true), (7.0, false)]
    } else {
        [(0.0, false), (5.0, false), (0.0, false), (7.0, false)]
    };

    let mut add = |start: f64, dur: f64, f: &mut dyn FnMut(f64) -> f32| {
        let s = (start * sr).round() as usize;
        for k in 0..(dur * sr) as usize {
            if let Some(v) = x.get_mut(s + k) {
                *v += f(k as f64 / sr);
            }
        }
    };

    for (i, &b) in beats.iter().enumerate() {
        let bar = i / 4;
        let in_bar = i % 4;
        let next = beats.get(i + 1).copied().unwrap_or(b + (b - beats[i.saturating_sub(1)]).max(0.3));
        let off = b + (next - b) / 2.0;
        let kick = spec.kickless_bars.is_none_or(|(a, z)| bar < a || bar >= z);

        if kick {
            let mut phase = 0.0f64;
            let mut click = Rng(rng.0 ^ 0x55);
            add(b, 0.35, &mut |t| {
                phase += 2.0 * std::f64::consts::PI * (45.0 + 105.0 * (-t * 30.0).exp()) / sr;
                let body = phase.sin() * (-t * 8.0).exp() * 0.7;
                let c = if t < 0.0015 { click.noise() as f64 * 0.3 } else { 0.0 };
                (body + c) as f32
            });
        }
        if in_bar % 2 == 1 {
            // Clap: band-limited noise.
            let (mut z1, mut z2) = (0.0f32, 0.0f32);
            let mut n = Rng(rng.0 ^ 0x77);
            add(b, 0.15, &mut |t| {
                z1 += 0.25 * (n.noise() - z1);
                z2 += 0.25 * (z1 - z2);
                (z1 - z2) * 2.0 * (-t * 40.0).exp() as f32 * 0.5
            });
        }
        // Offbeat hat.
        let mut prev = 0.0f32;
        let mut n = Rng(rng.0 ^ 0x99);
        add(off, 0.05, &mut |t| {
            let v = n.noise();
            let hp = v - prev;
            prev = v;
            hp * (-t * 120.0).exp() as f32 * 0.12
        });
        let (root, _) = progression[bar % 4];
        if spec.rolling_bass {
            // Sub notes on sixteenths 2–4, louder than the kick's body.
            let f = midi_hz(24.0 + tonic + root);
            let sixteenth = (next - b) / 4.0;
            for k in 1..4 {
                let mut phase = 0.0f64;
                add(b + k as f64 * sixteenth, sixteenth * 0.95, &mut |t| {
                    phase += 2.0 * std::f64::consts::PI * f / sr;
                    let swell = (t / 0.03).min(1.0);
                    let release = ((sixteenth * 0.95 - t) / 0.005).clamp(0.0, 1.0);
                    (phase.sin() * swell * release * 0.9) as f32
                });
            }
        } else {
            // Offbeat bass on the chord root.
            let f = midi_hz(36.0 + tonic + root);
            let mut lp = 0.0f32;
            add(off, 0.2, &mut |t| {
                let saw = (2.0 * (t * f).fract() - 1.0) as f32;
                lp += 0.1 * (saw - lp);
                lp * (-t * 6.0).exp() as f32 * 0.25
            });
        }
        if in_bar == 0 {
            // Pad chord for the bar.
            let (root, minor) = progression[bar % 4];
            let bar_len = beats.get(i + 4).copied().unwrap_or(b + 4.0 * (next - b)) - b;
            let notes = [0.0, if minor { 3.0 } else { 4.0 }, 7.0, 12.0].map(|iv| midi_hz(60.0 + tonic + root + iv));
            add(b, bar_len, &mut |t| {
                let fade = (t / 0.05).min(1.0).min(((bar_len - t) / 0.05).max(0.0)) as f32;
                let s: f64 = notes.iter().map(|f| (2.0 * std::f64::consts::PI * f * t).sin()).sum();
                (s * 0.04) as f32 * fade
            });
            if bar % 8 == 0 {
                let mut prev = 0.0f32;
                let mut n = Rng(rng.0 ^ 0x33);
                add(b, 1.5, &mut |t| {
                    let v = n.noise();
                    let hp = v - prev;
                    prev = v;
                    hp * (-t * 3.0).exp() as f32 * 0.12
                });
            }
        }
    }
    for v in x.iter_mut() {
        *v += rng.noise() * 1e-4;
    }
    let frames = x.iter().map(|v| [*v, *v]).collect();
    Rendered { audio: DecodedAudio { sample_rate: spec.sr, frames }, beats, pulse }
}
