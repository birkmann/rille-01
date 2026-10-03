//! Drum kits: the synthesized factory kits and the user's own, which are
//! folders of samples under `<data>/drums/kits/<kit name>/`.
//!
//! A user kit takes each instrument's sample from `kit.toml`
//! (`[samples]` with `BD = "my kick.wav"` …) or, without an entry there,
//! from a file named after the instrument (`BD.wav`, `kick.flac`,
//! `snare.mp3`, `hihat.wav` …, see [`ALIASES`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rille_core::drums::{INSTRUMENTS, NAMES};
use rille_engine::TrackAudio;
use serde::{Deserialize, Serialize};

use super::synth::{FACTORY_KITS, SAMPLE_RATE, factory_kit};

/// Samples are cut to this length (a whole track loaded by mistake stays
/// usable as a one-shot).
pub const MAX_SAMPLE_SECS: f64 = 8.0;
/// File names (without extension, any case) taken for each instrument.
pub const ALIASES: [&[&str]; INSTRUMENTS] = [
    &["bd", "kick", "bassdrum"],
    &["sd", "snare"],
    &["ch", "hh", "hihat", "hat", "closedhat", "closed"],
    &["oh", "openhat", "open"],
    &["cp", "clap"],
    &["rs", "rim", "rimshot"],
    &["lt", "tom", "lowtom"],
    &["cy", "cymbal", "crash", "ride"],
];
const AUDIO_EXTENSIONS: [&str; 7] = ["wav", "flac", "mp3", "ogg", "aif", "aiff", "m4a"];

pub type Samples = [Option<Arc<TrackAudio>>; INSTRUMENTS];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KitInfo {
    pub name: String,
    pub factory: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct KitFile {
    /// Instrument name (`BD` …) → file in the kit's folder.
    samples: BTreeMap<String, String>,
}

pub fn kits_dir(drums: &Path) -> PathBuf {
    drums.join("kits")
}

pub fn is_factory(name: &str) -> bool {
    FACTORY_KITS.contains(&name)
}

/// The factory kits, then the user's kits by name.
pub fn list(drums: &Path) -> Vec<KitInfo> {
    let mut v: Vec<KitInfo> = FACTORY_KITS.iter().map(|n| KitInfo { name: (*n).into(), factory: true }).collect();
    let mut user: Vec<String> = std::fs::read_dir(kits_dir(drums))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| !is_factory(n) && !n.starts_with('.'))
        .collect();
    user.sort_by_key(|n| n.to_lowercase());
    v.extend(user.into_iter().map(|name| KitInfo { name, factory: false }));
    v
}

/// A folder name for a kit called `name`.
pub fn folder_name(name: &str) -> String {
    let n: String =
        name.trim().chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':') { '_' } else { c }).collect();
    n.trim_start_matches('.').to_owned()
}

/// Loads the samples of kit `name` (decoding user samples: call off the UI
/// thread). Missing samples are `None`.
pub fn load(drums: &Path, name: &str) -> Result<Samples, String> {
    if let Some(i) = FACTORY_KITS.iter().position(|&n| n == name) {
        let kit = factory_kit(i).ok_or("no such factory kit")?;
        return Ok(kit.map(|mono| {
            let frames = mono.into_iter().map(|x| [x, x]).collect();
            Some(Arc::new(TrackAudio { sample_rate: SAMPLE_RATE, frames }))
        }));
    }
    let dir = kits_dir(drums).join(folder_name(name));
    if !dir.is_dir() {
        return Err(format!("drum kit \"{name}\" not found"));
    }
    let mut samples: Samples = Default::default();
    for (i, slot) in samples.iter_mut().enumerate() {
        if let Some(path) = sample_file(&dir, i) {
            match decode(&path) {
                Ok(a) => *slot = Some(Arc::new(a)),
                Err(e) => eprintln!("drum kit {name}: {}: {e}", path.display()),
            }
        }
    }
    Ok(samples)
}

/// The file kit folder `dir` plays for instrument `inst`.
pub fn sample_file(dir: &Path, inst: usize) -> Option<PathBuf> {
    let file: KitFile = std::fs::read_to_string(dir.join("kit.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).map_err(|e| eprintln!("{}: {e}", dir.display())).ok())
        .unwrap_or_default();
    if let Some(f) = file.samples.get(NAMES[inst]) {
        let p = dir.join(f);
        return p.is_file().then_some(p);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    files.sort();
    files.into_iter().find(|p| {
        let ext = p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
        let stem = p.file_stem().and_then(|s| s.to_str()).map(str::to_lowercase).unwrap_or_default();
        let stem: String = stem.chars().filter(|c| c.is_alphanumeric()).collect();
        AUDIO_EXTENSIONS.contains(&ext.as_str()) && ALIASES[inst].contains(&stem.as_str())
    })
}

/// Decodes a sample, cut to [`MAX_SAMPLE_SECS`] with a short fade.
pub fn decode(path: &Path) -> Result<TrackAudio, String> {
    let a = rille_decode::decode_file(path, None, &mut |_| {}).map_err(|e| e.to_string())?;
    Ok(trim(TrackAudio { sample_rate: a.sample_rate, frames: a.frames }))
}

fn trim(mut a: TrackAudio) -> TrackAudio {
    let max = (MAX_SAMPLE_SECS * f64::from(a.sample_rate)) as usize;
    if a.frames.len() > max {
        a.frames.truncate(max);
        let fade = (a.sample_rate / 100) as usize;
        let n = a.frames.len();
        for (i, f) in a.frames[n - fade.min(n)..].iter_mut().enumerate() {
            let g = 1.0 - i as f32 / fade as f32;
            f[0] *= g;
            f[1] *= g;
        }
    }
    a
}

fn write_wav(path: &Path, a: &TrackAudio) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: a.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for f in &a.frames {
        w.write_sample(f[0]).map_err(|e| e.to_string())?;
        w.write_sample(f[1]).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

/// Creates user kit `name` from `samples` (one WAV per instrument); returns
/// its name as listed.
pub fn create(drums: &Path, name: &str, samples: &Samples) -> Result<String, String> {
    let folder = folder_name(name);
    if folder.is_empty() || is_factory(&folder) {
        return Err(format!("\"{name}\" cannot be used as a kit name"));
    }
    let dir = kits_dir(drums).join(&folder);
    if dir.exists() {
        return Err(format!("a kit called \"{folder}\" already exists"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (i, s) in samples.iter().enumerate() {
        if let Some(a) = s {
            write_wav(&dir.join(format!("{}.wav", NAMES[i])), a)?;
        }
    }
    Ok(folder)
}

/// A kit name not taken yet, from `base`.
pub fn unique_name(drums: &Path, base: &str) -> String {
    let taken = |n: &str| is_factory(n) || kits_dir(drums).join(folder_name(n)).exists();
    if !taken(base) {
        return base.to_owned();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).expect("a free name")
}

/// Stores `audio` as instrument `inst`'s sample of user kit `name`.
pub fn set_sample(drums: &Path, name: &str, inst: usize, audio: &TrackAudio) -> Result<(), String> {
    let dir = kits_dir(drums).join(folder_name(name));
    remove_sample(drums, name, inst)?;
    write_wav(&dir.join(format!("{}.wav", NAMES[inst])), audio)
}

/// Removes instrument `inst`'s sample from user kit `name`.
pub fn remove_sample(drums: &Path, name: &str, inst: usize) -> Result<(), String> {
    let dir = kits_dir(drums).join(folder_name(name));
    if !dir.is_dir() || is_factory(name) {
        return Err(format!("\"{name}\" is not one of your kits"));
    }
    while let Some(p) = sample_file(&dir, inst) {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    // Drop a kit.toml entry pointing at it.
    let path = dir.join("kit.toml");
    if let Some(mut file) = std::fs::read_to_string(&path).ok().and_then(|t| toml::from_str::<KitFile>(&t).ok())
        && file.samples.remove(NAMES[inst]).is_some()
    {
        let text = toml::to_string_pretty(&file).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(len: usize) -> TrackAudio {
        TrackAudio { sample_rate: 44_100, frames: (0..len).map(|i| [i as f32 / len as f32, 0.0]).collect() }
    }

    #[test]
    fn factory_kits_load_and_user_kits_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let drums = dir.path();
        let names: Vec<String> = list(drums).into_iter().map(|k| k.name).collect();
        assert_eq!(names, FACTORY_KITS);
        let samples = load(drums, "909 Core").unwrap();
        assert!(samples.iter().all(Option::is_some));
        assert!(create(drums, "808 Boom", &samples).is_err(), "factory names are taken");

        let name = create(drums, "My/Kit", &samples).unwrap();
        assert_eq!(name, "My_Kit");
        assert!(create(drums, "My_Kit", &samples).is_err());
        assert_eq!(unique_name(drums, "My_Kit"), "My_Kit 2");
        let back = load(drums, &name).unwrap();
        for (a, b) in samples.iter().zip(&back) {
            assert_eq!(a.as_ref().unwrap().frames, b.as_ref().unwrap().frames);
        }
        let listed = list(drums);
        assert_eq!(listed.last(), Some(&KitInfo { name: name.clone(), factory: false }));

        remove_sample(drums, &name, 1).unwrap();
        assert!(load(drums, &name).unwrap()[1].is_none());
        set_sample(drums, &name, 1, &audio(100)).unwrap();
        assert_eq!(load(drums, &name).unwrap()[1].as_ref().unwrap().frames.len(), 100);
        assert!(remove_sample(drums, "909 Core", 0).is_err());
        assert!(load(drums, "nope").is_err());
    }

    #[test]
    fn samples_by_file_name_or_kit_file() {
        let dir = tempfile::tempdir().unwrap();
        let kit = kits_dir(dir.path()).join("Mine");
        std::fs::create_dir_all(&kit).unwrap();
        write_wav(&kit.join("Kick.wav"), &audio(10)).unwrap();
        write_wav(&kit.join("hi-hat.wav"), &audio(20)).unwrap();
        write_wav(&kit.join("other.wav"), &audio(30)).unwrap();
        assert_eq!(sample_file(&kit, 0), Some(kit.join("Kick.wav")));
        assert_eq!(sample_file(&kit, 2), Some(kit.join("hi-hat.wav")));
        assert_eq!(sample_file(&kit, 1), None);
        std::fs::write(kit.join("kit.toml"), "[samples]\nSD = \"other.wav\"\n").unwrap();
        assert_eq!(sample_file(&kit, 1), Some(kit.join("other.wav")));
        let s = load(dir.path(), "Mine").unwrap();
        assert_eq!(s[1].as_ref().unwrap().frames.len(), 30);
    }

    #[test]
    fn long_samples_are_cut() {
        let a = trim(audio(44_100 * 20));
        assert_eq!(a.frames.len(), 44_100 * 8);
        assert!(a.frames.last().unwrap()[0].abs() < 0.01);
    }
}
