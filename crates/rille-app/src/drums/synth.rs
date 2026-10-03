//! The factory drum kits, synthesized: nothing to ship or license, and the
//! same sounds on every machine (the noise is seeded).
//!
//! Analog-style recipes: a kick is a sine whose pitch falls, a snare two
//! tones and noise, hats six square waves at inharmonic ratios, a clap
//! band-passed noise struck a few times in quick succession.

use rille_core::drums::INSTRUMENTS;

pub const SAMPLE_RATE: u32 = 48_000;
const SR: f32 = SAMPLE_RATE as f32;

/// Names of the factory kits, in the order of the kit list.
pub const FACTORY_KITS: [&str; 3] = ["909 Core", "808 Boom", "Minimal"];

/// Sound design of one kit.
#[derive(Clone, Copy)]
struct KitParams {
    /// Kick: start and end pitch (Hz), pitch fall time, decay, click, drive.
    kick: (f32, f32, f32, f32, f32, f32),
    /// Snare: tone (Hz), tone decay, noise decay, noise share.
    snare: (f32, f32, f32, f32),
    /// Hats: pitch scale of the metal oscillators, closed and open decay,
    /// noise share.
    hat: (f32, f32, f32, f32),
    clap_decay: f32,
    /// Tom: pitch and decay.
    tom: (f32, f32),
    cymbal_decay: f32,
}

const KITS: [KitParams; 3] = [
    // 909: punchy kick with a click, bright snare, noisy hats.
    KitParams {
        kick: (260.0, 52.0, 0.035, 0.38, 0.5, 2.2),
        snare: (190.0, 0.06, 0.16, 0.65),
        hat: (1.0, 0.045, 0.32, 0.45),
        clap_decay: 0.14,
        tom: (125.0, 0.32),
        cymbal_decay: 1.1,
    },
    // 808: long, round kick, softer snare, pure metal hats.
    KitParams {
        kick: (110.0, 46.0, 0.06, 1.1, 0.12, 1.3),
        snare: (240.0, 0.09, 0.12, 0.45),
        hat: (0.92, 0.035, 0.42, 0.1),
        clap_decay: 0.2,
        tom: (100.0, 0.5),
        cymbal_decay: 1.6,
    },
    // Minimal: short and dry.
    KitParams {
        kick: (180.0, 50.0, 0.025, 0.28, 0.3, 3.0),
        snare: (220.0, 0.04, 0.08, 0.5),
        hat: (1.15, 0.025, 0.18, 0.3),
        clap_decay: 0.09,
        tom: (150.0, 0.2),
        cymbal_decay: 0.6,
    },
];

/// The samples of factory kit `index`, mono at [`SAMPLE_RATE`], one per
/// instrument (BD SD CH OH CP RS LT CY).
pub fn factory_kit(index: usize) -> Option<[Vec<f32>; INSTRUMENTS]> {
    let p = *KITS.get(index)?;
    let mut noise = Noise(0x9e37_79b9_7f4a_7c15 ^ index as u64);
    Some([
        kick(p),
        snare(p, &mut noise),
        hat(p, p.hat.1, &mut noise),
        hat(p, p.hat.2, &mut noise),
        clap(p, &mut noise),
        rim(),
        tom(p, &mut noise),
        cymbal(p, &mut noise),
    ])
}

/// Seeded white noise (xorshift).
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

/// State-variable filter (Chamberlin), for band- and high-passed noise.
struct Svf {
    f: f32,
    q: f32,
    low: f32,
    band: f32,
}

impl Svf {
    fn new(freq: f32, q: f32) -> Self {
        Self { f: 2.0 * (std::f32::consts::PI * freq.min(SR / 6.0) / SR).sin(), q: 1.0 / q, low: 0.0, band: 0.0 }
    }

    /// (low, band, high)
    fn tick(&mut self, x: f32) -> (f32, f32, f32) {
        self.low += self.f * self.band;
        let high = x - self.low - self.q * self.band;
        self.band += self.f * high;
        (self.low, self.band, high)
    }
}

fn frames(secs: f32) -> usize {
    (secs * SR) as usize
}

fn env(t: f32, tau: f32) -> f32 {
    (-t / tau).exp()
}

/// Peak at `peak`, a short fade at the end, and the tail trimmed once quiet.
fn finish(mut v: Vec<f32>, peak: f32) -> Vec<f32> {
    let max = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if max > 0.0 {
        v.iter_mut().for_each(|x| *x *= peak / max);
    }
    if let Some(last) = v.iter().rposition(|x| x.abs() > 1e-4) {
        v.truncate(last + 1);
    }
    let fade = frames(0.004).min(v.len());
    let n = v.len();
    for (i, x) in v[n - fade..].iter_mut().enumerate() {
        *x *= 1.0 - i as f32 / fade as f32;
    }
    v
}

fn kick(p: KitParams) -> Vec<f32> {
    let (f0, f1, fall, decay, click, drive) = p.kick;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(frames(decay * 5.0));
    for i in 0..frames(decay * 5.0) {
        let t = i as f32 / SR;
        let f = f1 + (f0 - f1) * env(t, fall);
        phase += f / SR;
        let body = (phase * std::f32::consts::TAU).sin() * env(t, decay);
        // A short high blip for the attack.
        let blip = (t * 1800.0 * std::f32::consts::TAU).sin() * env(t, 0.0025) * click;
        out.push(((body + blip) * drive).tanh() / drive.tanh());
    }
    finish(out, 0.95)
}

fn snare(p: KitParams, noise: &mut Noise) -> Vec<f32> {
    let (tone, tone_decay, noise_decay, share) = p.snare;
    let mut hp = Svf::new(1800.0, 0.8);
    let len = frames(noise_decay.max(tone_decay) * 5.0);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let t = i as f32 / SR;
        let w = std::f32::consts::TAU * t;
        let tones = ((w * tone).sin() + 0.6 * (w * tone * 1.74).sin()) * env(t, tone_decay);
        let (_, _, n) = hp.tick(noise.next());
        out.push(tones * (1.0 - share) + n * env(t, noise_decay) * share * 1.6);
    }
    finish(out, 0.9)
}

/// Six square waves at the ratios of analog metal circuits.
fn metal(t: f32, scale: f32) -> f32 {
    const HZ: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
    HZ.iter().map(|f| if (t * f * scale * 1.0).fract() < 0.5 { 1.0 } else { -1.0 }).sum::<f32>() / 6.0
}

fn hat(p: KitParams, decay: f32, noise: &mut Noise) -> Vec<f32> {
    let (scale, _, _, share) = p.hat;
    let mut band = Svf::new(10_000.0, 1.2);
    let mut hp = Svf::new(7_000.0, 0.7);
    let len = frames(decay * 6.0);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let t = i as f32 / SR;
        // The metal runs much faster: the hats' ratios are of the high partials.
        let x = metal(t * 16.0, scale) * (1.0 - share) + noise.next() * share;
        let (_, b, _) = band.tick(x);
        let (_, _, h) = hp.tick(b);
        out.push(h * env(t, decay));
    }
    finish(out, 0.75)
}

fn clap(p: KitParams, noise: &mut Noise) -> Vec<f32> {
    let mut bp = Svf::new(1150.0, 1.6);
    let len = frames(0.03 + p.clap_decay * 5.0);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let t = i as f32 / SR;
        // Three quick slaps, then the room.
        let slaps = [0.0, 0.011, 0.022].iter().filter(|&&s| t >= s).map(|&s| env(t - s, 0.0035)).sum::<f32>();
        let tail = if t >= 0.03 { env(t - 0.03, p.clap_decay) } else { 0.0 };
        let (_, b, _) = bp.tick(noise.next());
        out.push(b * (slaps + tail));
    }
    finish(out, 0.9)
}

fn rim() -> Vec<f32> {
    let len = frames(0.08);
    let out = (0..len)
        .map(|i| {
            let t = i as f32 / SR;
            let w = std::f32::consts::TAU * t;
            ((w * 1700.0).sin() * 0.6 + (w * 455.0).sin()) * env(t, 0.011)
        })
        .collect();
    finish(out, 0.8)
}

fn tom(p: KitParams, noise: &mut Noise) -> Vec<f32> {
    let (f, decay) = p.tom;
    let mut phase = 0.0f32;
    let len = frames(decay * 5.0);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let t = i as f32 / SR;
        phase += (f * (1.0 + 0.5 * env(t, 0.05))) / SR;
        let body = (phase * std::f32::consts::TAU).sin() * env(t, decay);
        out.push(body + noise.next() * env(t, 0.01) * 0.15);
    }
    finish(out, 0.9)
}

fn cymbal(p: KitParams, noise: &mut Noise) -> Vec<f32> {
    let mut hp = Svf::new(5_000.0, 0.7);
    let len = frames(p.cymbal_decay * 4.0);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let t = i as f32 / SR;
        let x = metal(t * 24.0, p.hat.0 * 1.07) * 0.6 + noise.next() * 0.4;
        let (_, _, h) = hp.tick(x);
        out.push(h * env(t, p.cymbal_decay));
    }
    finish(out, 0.7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_kits_sound_and_repeat() {
        assert_eq!(FACTORY_KITS.len(), KITS.len());
        for k in 0..FACTORY_KITS.len() {
            let kit = factory_kit(k).unwrap();
            for (i, s) in kit.iter().enumerate() {
                let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                assert!(peak > 0.5 && peak <= 1.0, "kit {k} instrument {i}: peak {peak}");
                assert!(s.len() > frames(0.02) && s.len() < frames(8.0), "kit {k} instrument {i}: {}", s.len());
                assert!(s.iter().all(|x| x.is_finite()));
                assert!(s.last().unwrap().abs() < 1e-3, "ends quietly");
            }
            assert_eq!(factory_kit(k).unwrap(), kit, "deterministic");
        }
        assert!(factory_kit(FACTORY_KITS.len()).is_none());
    }
}
