use std::path::{Path, PathBuf};

use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

const BRIDGES: &[&str] =
    &["src/app_controller.rs", "src/deck_controller.rs", "src/waveform.rs", "src/models.rs", "src/tree_model.rs"];
const SINGLETONS: &[&str] = &["Theme.qml"];
/// Bundled fonts (SIL OFL, see `assets/fonts/OFL.txt`), loaded by `Theme.qml`
/// from `qrc:/qt/qml/rille/ui/assets/fonts/`.
const FONTS: &[&str] = &[
    "assets/fonts/Geist-Regular.ttf",
    "assets/fonts/Geist-Medium.ttf",
    "assets/fonts/Geist-SemiBold.ttf",
    "assets/fonts/Geist-Bold.ttf",
    "assets/fonts/GeistMono-Regular.ttf",
    "assets/fonts/GeistMono-Medium.ttf",
];

/// Every `.qml` file under `qml/`, sorted, relative to the crate.
fn qml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("qml dir").flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            qml_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "qml") {
            out.push(p);
        }
    }
}

fn main() {
    println!("cargo::rerun-if-changed=qml");
    let mut files = Vec::new();
    qml_files(Path::new("qml"), &mut files);
    let module = QmlModule::new("rille.ui").qml_files(files.into_iter().map(|p| {
        let singleton = p.file_name().is_some_and(|n| SINGLETONS.iter().any(|s| n == *s));
        QmlFile::from(p).singleton(singleton)
    }));
    let builder = CxxQtBuilder::new_qml_module(module)
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .files(BRIDGES)
        .qrc_resources(FONTS.iter().copied());
    // SAFETY: only silences a GCC 16 warning triggered inside Qt headers; it
    // does not change how anything is compiled or linked.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.flag_if_supported("-Wno-sfinae-incomplete");
        })
    };
    builder.build();
}
