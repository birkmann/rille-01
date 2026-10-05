//! Drum kits: the synthesized factory kits and the user's own, which are
//! folders of samples under `<data>/drums/kits/<kit name>/`.
//!
//! A user kit takes each instrument's sample from `kit.toml`
//! (`[samples]` with `BD = "my kick.wav"` …) or, without an entry there,
//! from a file named after the instrument (`BD.wav`, `kick.flac`,
//! `snare.mp3`, `hihat.wav` …, see [`ALIASES`]).
//!
//! Saved as a rack, `kit.toml` also keeps the track order, the sounds'
//! names and each instrument's level, tune and decay:
//!
//! ```toml
//! order = ["BD", "CP", "SD", "CH", "OH", "RS", "LT", "CY"]
//! [names]
//! BD = "Kick 04"
//! [sounds.BD]
//! level = 0.9
//! tune = 0.5
//! decay = 1.0
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rille_core::drums::{INSTRUMENTS, NAMES, Order, order_from_names, order_names};
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
/// Instruments in the order file names are matched against [`ALIASES`]:
/// "open hi-hat" is the open hi-hat, not the closed one.
const MATCH_ORDER: [usize; INSTRUMENTS] = [3, 2, 0, 1, 4, 5, 6, 7];

pub type Samples = [Option<Arc<TrackAudio>>; INSTRUMENTS];
/// What each sound is called (the file or track it came from).
pub type Labels = [Option<String>; INSTRUMENTS];

/// Level, tune and decay of one instrument, as a rack keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sound {
    pub level: f32,
    pub tune: f32,
    pub decay: f32,
}

impl Default for Sound {
    fn default() -> Self {
        Self { level: 1.0, tune: 0.5, decay: 1.0 }
    }
}

/// A kit as loaded: its samples and their names, and what was saved with
/// it as a rack.
#[derive(Clone, Default)]
pub struct Kit {
    pub samples: Samples,
    pub labels: Labels,
    pub order: Option<Order>,
    pub sounds: Option<[Sound; INSTRUMENTS]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KitInfo {
    pub name: String,
    pub factory: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct KitFile {
    /// Instrument names, left to right.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    order: Vec<String>,
    /// Instrument name (`BD` …) → file in the kit's folder.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    samples: BTreeMap<String, String>,
    /// Instrument name → what its sound is called.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    names: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    sounds: BTreeMap<String, Sound>,
}

impl KitFile {
    fn read(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join("kit.toml"))
            .ok()
            .and_then(|t| toml::from_str(&t).map_err(|e| eprintln!("{}: {e}", dir.display())).ok())
            .unwrap_or_default()
    }

    fn write(&self, dir: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("kit.toml"), text).map_err(|e| e.to_string())
    }

    fn set_rack(&mut self, order: &Order, sounds: &[Sound; INSTRUMENTS]) {
        self.order = order_names(order);
        self.sounds = (0..INSTRUMENTS).map(|i| (NAMES[i].to_owned(), sounds[i])).collect();
    }
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

/// Loads kit `name` (decoding user samples: call off the UI thread).
/// Missing samples are `None`.
pub fn load(drums: &Path, name: &str) -> Result<Kit, String> {
    if let Some(i) = FACTORY_KITS.iter().position(|&n| n == name) {
        let kit = factory_kit(i).ok_or("no such factory kit")?;
        let samples = kit.map(|mono| {
            let frames = mono.into_iter().map(|x| [x, x]).collect();
            Some(Arc::new(TrackAudio { sample_rate: SAMPLE_RATE, frames }))
        });
        return Ok(Kit { samples, ..Kit::default() });
    }
    let dir = kits_dir(drums).join(folder_name(name));
    if !dir.is_dir() {
        return Err(format!("drum kit \"{name}\" not found"));
    }
    let file = KitFile::read(&dir);
    let mut kit = Kit::default();
    for (i, inst_name) in NAMES.iter().enumerate() {
        let Some(path) = sample_file(&dir, i) else { continue };
        match decode(&path) {
            Ok(a) => kit.samples[i] = Some(Arc::new(a)),
            Err(e) => {
                eprintln!("drum kit {name}: {}: {e}", path.display());
                continue;
            }
        }
        // Without a name, the file's (unless it is just the instrument's).
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        kit.labels[i] = file
            .names
            .get(*inst_name)
            .cloned()
            .or_else(|| (!stem.eq_ignore_ascii_case(inst_name)).then(|| stem.to_owned()));
    }
    kit.order = (!file.order.is_empty()).then(|| order_from_names(&file.order));
    kit.sounds = (!file.sounds.is_empty()).then(|| {
        std::array::from_fn(|i| {
            let s = file.sounds.get(NAMES[i]).copied().unwrap_or_default();
            Sound { level: s.level.clamp(0.0, 1.0), tune: s.tune.clamp(0.0, 1.0), decay: s.decay.clamp(0.0, 1.0) }
        })
    });
    Ok(kit)
}

/// The file kit folder `dir` plays for instrument `inst`.
pub fn sample_file(dir: &Path, inst: usize) -> Option<PathBuf> {
    if let Some(f) = KitFile::read(dir).samples.get(NAMES[inst]) {
        let p = dir.join(f);
        return p.is_file().then_some(p);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    files.sort();
    files.into_iter().find(|p| {
        let stem = p.file_stem().and_then(|s| s.to_str()).map(str::to_lowercase).unwrap_or_default();
        let stem: String = stem.chars().filter(|c| c.is_alphanumeric()).collect();
        is_audio(p) && ALIASES[inst].contains(&stem.as_str())
    })
}

fn is_audio(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
    AUDIO_EXTENSIONS.contains(&ext.as_str())
}

/// The instrument a sound's file name suggests: a word of it (or all of it)
/// is one of the [`ALIASES`], as in `Kick 02.wav`, `808_snare.flac` or
/// `Hi-Hat Open.wav`.
pub fn instrument_for(path: &Path) -> Option<usize> {
    let stem = path.file_stem()?.to_str()?.to_lowercase();
    let mut words: Vec<String> = Vec::new();
    let mut prev: Option<char> = None;
    for c in stem.chars() {
        match prev {
            _ if !c.is_alphanumeric() => {
                prev = None;
                continue;
            }
            Some(p) if p.is_ascii_digit() == c.is_ascii_digit() => words.last_mut().expect("a word").push(c),
            _ => words.push(c.to_string()),
        }
        prev = Some(c);
    }
    words.push(words.concat());
    MATCH_ORDER.into_iter().find(|&i| ALIASES[i].iter().any(|a| words.iter().any(|w| w == a)))
}

/// Which instrument each of `files` becomes when they are loaded together.
/// `into`: the first goes to that instrument, the others to the tracks after
/// it (in `order`); `None`: each to the instrument its name suggests (see
/// [`instrument_for`]), the rest to the tracks left, left to right. Also
/// returns the number of files there was no track for.
pub fn assign(files: &[PathBuf], order: &Order, into: Option<usize>) -> (Vec<(usize, PathBuf)>, usize) {
    let mut out: Vec<(usize, PathBuf)> = Vec::new();
    let mut rest: Vec<&PathBuf> = Vec::new();
    let mut tracks: Vec<usize> = order.iter().map(|&i| usize::from(i)).collect();
    match into {
        Some(inst) => {
            let from = tracks.iter().position(|&i| i == inst).unwrap_or(0);
            tracks.drain(..from);
            rest.extend(files);
        }
        None => {
            for f in files {
                match instrument_for(f).filter(|i| !out.iter().any(|(j, _)| j == i)) {
                    Some(i) => out.push((i, f.clone())),
                    None => rest.push(f),
                }
            }
            tracks.retain(|i| !out.iter().any(|(j, _)| j == i));
        }
    }
    let left = rest.len().saturating_sub(tracks.len());
    out.extend(tracks.into_iter().zip(rest).map(|(i, f)| (i, f.clone())));
    (out, left)
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

/// Writes `kit` into the new folder `dir`: one WAV per instrument and a
/// `kit.toml` with the names, the order and the sounds.
fn write_kit(dir: &Path, kit: &Kit) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut file = KitFile::default();
    for (i, s) in kit.samples.iter().enumerate() {
        if let Some(a) = s {
            write_wav(&dir.join(format!("{}.wav", NAMES[i])), a)?;
            if let Some(l) = &kit.labels[i] {
                file.names.insert(NAMES[i].to_owned(), l.clone());
            }
        }
    }
    if let (Some(order), Some(sounds)) = (&kit.order, &kit.sounds) {
        file.set_rack(order, sounds);
    }
    file.write(dir)
}

/// Creates user kit `name` from `kit`; returns its name as listed.
pub fn create(drums: &Path, name: &str, kit: &Kit) -> Result<String, String> {
    let folder = folder_name(name);
    if folder.is_empty() || is_factory(&folder) {
        return Err(format!("\"{name}\" cannot be used as a kit name"));
    }
    let dir = kits_dir(drums).join(&folder);
    if dir.exists() {
        return Err(format!("a kit called \"{folder}\" already exists"));
    }
    write_kit(&dir, kit)?;
    Ok(folder)
}

/// Saves the track order and the sounds with user kit `name`.
pub fn save_rack(drums: &Path, name: &str, order: &Order, sounds: &[Sound; INSTRUMENTS]) -> Result<(), String> {
    let dir = user_kit_dir(drums, name)?;
    let mut file = KitFile::read(&dir);
    file.set_rack(order, sounds);
    file.write(&dir)
}

/// Writes `kit` (called `name`) as a folder of its own in `dest`, ready to
/// be imported elsewhere; returns the folder.
pub fn export(dest: &Path, name: &str, kit: &Kit) -> Result<PathBuf, String> {
    if !dest.is_dir() {
        return Err(format!("{} is not a folder", dest.display()));
    }
    let base = match folder_name(name) {
        n if n.is_empty() => "Drum rack".to_owned(),
        n => n,
    };
    let dir = std::iter::once(base.clone())
        .chain((2..).map(|i| format!("{base} {i}")))
        .map(|n| dest.join(n))
        .find(|d| !d.exists())
        .expect("a free name");
    write_kit(&dir, kit)?;
    Ok(dir)
}

/// Copies the kit in folder `src` (an exported rack, or any folder of
/// sounds named after the instruments) to the user's kits; returns its name.
pub fn import(drums: &Path, src: &Path) -> Result<String, String> {
    let files: Vec<(usize, PathBuf)> = (0..INSTRUMENTS).filter_map(|i| Some((i, sample_file(src, i)?))).collect();
    if files.is_empty() {
        return Err(format!(
            "no drum sounds in {}: name them after the instruments (BD.wav, kick.wav, snare.wav …)",
            src.display()
        ));
    }
    let base = src.file_name().and_then(|n| n.to_str()).unwrap_or("Imported kit");
    let name = folder_name(&unique_name(drums, base));
    let dir = kits_dir(drums).join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut file = KitFile::read(src);
    file.samples.clear();
    for (i, f) in &files {
        let file_name = f.file_name().and_then(|n| n.to_str()).expect("a file name");
        let to = dir.join(file_name);
        if !to.exists() {
            std::fs::copy(f, &to).map_err(|e| format!("{}: {e}", f.display()))?;
        }
        file.samples.insert(NAMES[*i].to_owned(), file_name.to_owned());
    }
    file.write(&dir)?;
    Ok(name)
}

fn user_kit_dir(drums: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = kits_dir(drums).join(folder_name(name));
    if !dir.is_dir() || is_factory(name) {
        return Err(format!("\"{name}\" is not one of your kits"));
    }
    Ok(dir)
}

/// A kit name not taken yet, from `base`.
pub fn unique_name(drums: &Path, base: &str) -> String {
    let taken = |n: &str| is_factory(n) || kits_dir(drums).join(folder_name(n)).exists();
    if !taken(base) {
        return base.to_owned();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).expect("a free name")
}

/// Stores `audio` as instrument `inst`'s sample of user kit `name`, called
/// `label`.
pub fn set_sample(
    drums: &Path,
    name: &str,
    inst: usize,
    audio: &TrackAudio,
    label: Option<&str>,
) -> Result<(), String> {
    let dir = user_kit_dir(drums, name)?;
    remove_sample(drums, name, inst)?;
    write_wav(&dir.join(format!("{}.wav", NAMES[inst])), audio)?;
    if let Some(l) = label {
        let mut file = KitFile::read(&dir);
        file.names.insert(NAMES[inst].to_owned(), l.to_owned());
        file.write(&dir)?;
    }
    Ok(())
}

/// Removes instrument `inst`'s sample from user kit `name`.
pub fn remove_sample(drums: &Path, name: &str, inst: usize) -> Result<(), String> {
    let dir = user_kit_dir(drums, name)?;
    while let Some(p) = sample_file(&dir, inst) {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    // Drop kit.toml entries pointing at it.
    if dir.join("kit.toml").is_file() {
        let mut file = KitFile::read(&dir);
        let had = file.samples.remove(NAMES[inst]).is_some() | file.names.remove(NAMES[inst]).is_some();
        if had {
            file.write(&dir)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rille_core::drums::DEFAULT_ORDER;

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
        let kit = load(drums, "909 Core").unwrap();
        assert!(kit.samples.iter().all(Option::is_some));
        assert!(kit.order.is_none() && kit.sounds.is_none() && kit.labels.iter().all(Option::is_none));
        assert!(create(drums, "808 Boom", &kit).is_err(), "factory names are taken");

        let name = create(drums, "My/Kit", &kit).unwrap();
        assert_eq!(name, "My_Kit");
        assert!(create(drums, "My_Kit", &kit).is_err());
        assert_eq!(unique_name(drums, "My_Kit"), "My_Kit 2");
        let back = load(drums, &name).unwrap();
        for (a, b) in kit.samples.iter().zip(&back.samples) {
            assert_eq!(a.as_ref().unwrap().frames, b.as_ref().unwrap().frames);
        }
        assert!(back.labels.iter().all(Option::is_none), "BD.wav is just the instrument");
        let listed = list(drums);
        assert_eq!(listed.last(), Some(&KitInfo { name: name.clone(), factory: false }));

        remove_sample(drums, &name, 1).unwrap();
        assert!(load(drums, &name).unwrap().samples[1].is_none());
        set_sample(drums, &name, 1, &audio(100), Some("Snare 07")).unwrap();
        let back = load(drums, &name).unwrap();
        assert_eq!(back.samples[1].as_ref().unwrap().frames.len(), 100);
        assert_eq!(back.labels[1].as_deref(), Some("Snare 07"));
        remove_sample(drums, &name, 1).unwrap();
        assert!(!std::fs::read_to_string(kits_dir(drums).join(&name).join("kit.toml")).unwrap().contains("Snare"));
        assert!(remove_sample(drums, "909 Core", 0).is_err());
        assert!(save_rack(drums, "909 Core", &DEFAULT_ORDER, &[Sound::default(); INSTRUMENTS]).is_err());
        assert!(load(drums, "nope").is_err());
    }

    #[test]
    fn racks_keep_order_names_and_sounds() {
        let dir = tempfile::tempdir().unwrap();
        let drums = dir.path();
        let mut kit = load(drums, "808 Boom").unwrap();
        kit.labels[0] = Some("Kick 04".into());
        kit.samples[7] = None;
        let order = [4, 0, 1, 2, 3, 5, 6, 7];
        let mut sounds = [Sound::default(); INSTRUMENTS];
        sounds[2] = Sound { level: 0.5, tune: 0.75, decay: 0.25 };
        kit.order = Some(order);
        kit.sounds = Some(sounds);
        let name = create(drums, "Rack", &kit).unwrap();
        let back = load(drums, &name).unwrap();
        assert_eq!((back.order, back.sounds), (Some(order), Some(sounds)));
        assert_eq!(back.labels[0].as_deref(), Some("Kick 04"));
        assert!(back.samples[7].is_none());

        sounds[2].tune = 0.5;
        save_rack(drums, &name, &DEFAULT_ORDER, &sounds).unwrap();
        let back = load(drums, &name).unwrap();
        assert_eq!((back.order, back.sounds), (Some(DEFAULT_ORDER), Some(sounds)));
        assert_eq!(back.labels[0].as_deref(), Some("Kick 04"), "saving the rack keeps the names");

        // Exported somewhere else and imported again: the same rack.
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let exported = export(&out, &name, &back).unwrap();
        assert_eq!(exported, out.join("Rack"));
        assert_eq!(export(&out, &name, &back).unwrap(), out.join("Rack 2"), "never overwrites");
        let imported = import(drums, &exported).unwrap();
        assert_eq!(imported, "Rack 2");
        let again = load(drums, &imported).unwrap();
        assert_eq!((again.order, again.sounds, again.labels.clone()), (back.order, back.sounds, back.labels.clone()));
        for (a, b) in back.samples.iter().zip(&again.samples) {
            assert_eq!(a.as_ref().map(|a| a.frames.len()), b.as_ref().map(|b| b.frames.len()));
        }
        assert!(import(drums, &out).is_err(), "a folder without sounds");
    }

    #[test]
    fn importing_a_plain_sample_folder() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("Pack");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        write_wav(&src.join("Kick.wav"), &audio(10)).unwrap();
        write_wav(&src.join("sub/x.wav"), &audio(20)).unwrap();
        std::fs::write(src.join("kit.toml"), "[samples]\nSD = \"sub/x.wav\"\n").unwrap();
        let name = import(dir.path(), &src).unwrap();
        let kit = load(dir.path(), &name).unwrap();
        assert_eq!(kit.samples[0].as_ref().unwrap().frames.len(), 10);
        assert_eq!(kit.samples[1].as_ref().unwrap().frames.len(), 20);
        assert_eq!(kit.labels[0].as_deref(), Some("Kick"));
    }

    #[test]
    fn instruments_from_file_names() {
        let i = |n: &str| instrument_for(Path::new(n));
        assert_eq!(i("Kick 02.wav"), Some(0));
        assert_eq!(i("808_snare.flac"), Some(1));
        assert_eq!(i("SD.wav"), Some(1));
        assert_eq!(i("Hi-Hat Closed.wav"), Some(2));
        assert_eq!(i("hihat01.wav"), Some(2));
        assert_eq!(i("Hi-Hat Open.wav"), Some(3));
        assert_eq!(i("OpenHat.wav"), Some(3));
        assert_eq!(i("clap3.aif"), Some(4));
        assert_eq!(i("Ride.wav"), Some(7));
        assert_eq!(i("chord.wav"), None);
        assert_eq!(i("perc 1.wav"), None);
    }

    #[test]
    fn several_files_find_their_tracks() {
        let files: Vec<PathBuf> = ["a.wav", "snare.wav", "b.wav", "kick.wav", "kick 2.wav"].map(PathBuf::from).into();
        let names = |v: &[(usize, PathBuf)]| -> Vec<(usize, String)> {
            v.iter().map(|(i, f)| (*i, f.to_string_lossy().into_owned())).collect()
        };
        // By name, the rest left to right on the free tracks.
        let order = [4, 0, 1, 2, 3, 5, 6, 7];
        let (v, left) = assign(&files, &order, None);
        assert_eq!(
            names(&v),
            [(1, "snare.wav"), (0, "kick.wav"), (4, "a.wav"), (2, "b.wav"), (3, "kick 2.wav")]
                .map(|(i, f)| (i, f.to_owned()))
        );
        assert_eq!(left, 0);
        // From a track on, in the order shown.
        let (v, left) = assign(&files, &order, Some(5));
        assert_eq!(names(&v).iter().map(|(i, _)| *i).collect::<Vec<_>>(), [5, 6, 7]);
        assert_eq!(left, 2);
        let (v, _) = assign(&files[..1], &order, Some(3));
        assert_eq!(names(&v), [(3, "a.wav".to_owned())]);
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
        assert_eq!(s.samples[1].as_ref().unwrap().frames.len(), 30);
    }

    #[test]
    fn long_samples_are_cut() {
        let a = trim(audio(44_100 * 20));
        assert_eq!(a.frames.len(), 44_100 * 8);
        assert!(a.frames.last().unwrap()[0].abs() < 0.01);
    }
}
