//! Loading, saving and choosing mapping files.

use crate::mapping::{Mapping, MappingError};
use regex::Regex;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct StoredMapping {
    pub mapping: Mapping,
    pub path: PathBuf,
    device: Regex,
}

/// All mappings found in a list of directories, in priority order.
#[derive(Clone, Debug, Default)]
pub struct MappingStore {
    entries: Vec<StoredMapping>,
    errors: Vec<(PathBuf, MappingError)>,
}

impl MappingStore {
    /// Loads every `*.toml` in `dirs`. Earlier directories take precedence,
    /// so pass the user directory (`~/.config/rille/mappings`) before the
    /// bundled one. Missing directories are skipped; broken files are listed
    /// in [`errors`](Self::errors).
    pub fn load<P: AsRef<Path>>(dirs: &[P]) -> Self {
        let mut store = Self::default();
        let mut loaded = Vec::new();
        for dir in dirs {
            let Ok(rd) = fs::read_dir(dir) else { continue };
            let mut files: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "toml") && p.is_file())
                .collect();
            files.sort();
            for path in files {
                match load_file(&path) {
                    Ok(mapping) => loaded.push((mapping, path)),
                    Err(e) => store.errors.push((path, e)),
                }
            }
        }
        let resolved: Vec<_> = loaded
            .iter()
            .map(|(m, path)| (resolve_includes(m, &loaded).and_then(|m| m.validate().map(|()| m)), path.clone()))
            .collect();
        for (mapping, path) in resolved {
            match mapping {
                Ok(m) => store.push(m, path),
                Err(e) => store.errors.push((path, e)),
            }
        }
        store
    }

    /// Adds a mapping with lowest priority, once per deck layout (see
    /// [`Mapping::variants`]). Invalid device regexes are ignored.
    pub fn push(&mut self, mapping: Mapping, path: PathBuf) {
        let Ok(device) = mapping.device_regex() else { return };
        for mapping in mapping.variants() {
            self.entries.push(StoredMapping { mapping, path: path.clone(), device: device.clone() });
        }
    }

    pub fn entries(&self) -> &[StoredMapping] {
        &self.entries
    }

    pub fn errors(&self) -> &[(PathBuf, MappingError)] {
        &self.errors
    }

    /// The first mapping whose device regex matches `port`; for a mapping with
    /// deck layouts, its first layout.
    pub fn find(&self, port: &str) -> Option<&Mapping> {
        self.entries.iter().find(|e| e.device.is_match(port)).map(|e| &e.mapping)
    }

    /// Every mapping (and deck layout) whose device regex matches `port`, in
    /// priority order.
    pub fn find_all<'a>(&'a self, port: &'a str) -> impl Iterator<Item = &'a Mapping> + 'a {
        self.entries.iter().filter(move |e| e.device.is_match(port)).map(|e| &e.mapping)
    }

    pub fn by_name(&self, name: &str) -> Option<&Mapping> {
        self.entries.iter().find(|e| e.mapping.name == name).map(|e| &e.mapping)
    }
}

/// `mapping` with the bindings of every mapping it includes appended; the
/// first mapping of that name in `all` wins, so a user copy overrides the
/// bundled one. Includes of included mappings are ignored.
fn resolve_includes(mapping: &Mapping, all: &[(Mapping, PathBuf)]) -> Result<Mapping, MappingError> {
    let mut m = mapping.clone();
    for name in std::mem::take(&mut m.include) {
        let (other, _) = all
            .iter()
            .find(|(o, _)| o.name == name)
            .ok_or_else(|| MappingError::Layout(format!("included mapping '{name}' not found")))?;
        m.inputs.extend(other.inputs.iter().cloned());
        m.outputs.extend(other.outputs.iter().cloned());
    }
    Ok(m)
}

pub fn load_file(path: &Path) -> Result<Mapping, MappingError> {
    let text = fs::read_to_string(path).map_err(|e| MappingError::Io(e.to_string()))?;
    Mapping::from_toml(&text)
}

/// Writes the mapping to `path`, creating parent directories.
pub fn save(mapping: &Mapping, path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, mapping.to_toml())
}

/// Saves into `dir` under a file name derived from the mapping name.
pub fn save_to_dir(mapping: &Mapping, dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join(format!("{}.toml", file_stem(&mapping.name)));
    save(mapping, &path)?;
    Ok(path)
}

fn file_stem(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "mapping".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rille-midi-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn user_dir_wins_and_regex_selects() {
        let root = tmp_dir("store");
        let (user, bundled) = (root.join("user"), root.join("bundled"));
        save(&Mapping::new("Bundled DDJ", "(?i)ddj-400"), &bundled.join("ddj.toml")).unwrap();
        save(&Mapping::new("Bundled generic", ".*"), &bundled.join("zz-generic.toml")).unwrap();
        let p = save_to_dir(&Mapping::new("My DDJ-400!", "DDJ-400"), &user).unwrap();
        assert_eq!(p.file_name().unwrap(), "my-ddj-400.toml");
        fs::write(bundled.join("broken.toml"), "name = 1").unwrap();
        fs::write(bundled.join("notes.txt"), "ignored").unwrap();

        let store = MappingStore::load(&[&user, &bundled, &root.join("missing")]);
        assert_eq!(store.entries().len(), 3);
        assert_eq!(store.errors().len(), 1);
        assert_eq!(store.find("DDJ-400:DDJ-400 MIDI 1 24:0").unwrap().name, "My DDJ-400!");
        assert_eq!(store.find("ddj-400 lowercase").unwrap().name, "Bundled DDJ");
        assert_eq!(store.find("Something else").unwrap().name, "Bundled generic");
        assert!(store.by_name("Bundled DDJ").is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn includes_merge_bindings_under_the_including_layouts() {
        use crate::mapping::{InputBinding, MidiSpec};
        let root = tmp_dir("include");
        let mut pad = Mapping::new("Pad", "(?i)pad");
        pad.deck_layouts = vec!["AB".into()];
        pad.inputs.push(InputBinding::new(
            "deck.B.play".parse::<rille_core::ControlTarget>().unwrap(),
            MidiSpec::Note { channel: 15, number: 1 },
        ));
        save(&pad, &root.join("pad.toml")).unwrap();
        let mut mixer = Mapping::new("Mixer", "(?i)mixer");
        mixer.deck_layouts = vec!["CD".into()];
        mixer.include = vec!["Pad".into()];
        mixer.inputs.push(InputBinding::new(
            "deck.A.volume".parse::<rille_core::ControlTarget>().unwrap(),
            MidiSpec::Cc { channel: 16, number: 0 },
        ));
        save(&mixer, &root.join("mixer.toml")).unwrap();
        let mut broken = Mapping::new("Broken", "x");
        broken.include = vec!["Missing".into()];
        save(&broken, &root.join("broken.toml")).unwrap();

        let store = MappingStore::load(&[&root]);
        assert_eq!(store.errors().len(), 1, "{:?}", store.errors());
        let m = store.find("Mixer MIDI 1").unwrap();
        assert_eq!(m.name, "Mixer (CD)");
        assert!(m.include.is_empty(), "resolved, so a saved copy stands alone");
        let targets: Vec<String> = m.inputs.iter().map(|b| b.target.to_string()).collect();
        assert_eq!(targets, ["deck.C.volume", "deck.D.play"]);
        assert_eq!(store.by_name("Pad (AB)").unwrap().inputs.len(), 1, "the included mapping is unchanged");
        let _ = fs::remove_dir_all(&root);
    }
}
