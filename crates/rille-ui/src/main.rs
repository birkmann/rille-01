//! Application entry point: starts the app core, then the QML UI.

mod app_controller;
mod deck_controller;
mod global;
mod models;
mod power;
mod startup;
mod tree_model;
mod waveform;

use std::path::PathBuf;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use rille_app::{App, Paths, StartOptions};

const MAIN_QML: &str = "qrc:/qt/qml/rille/ui/qml/Main.qml";

/// Bundled controller mappings: next to an installed binary
/// (`../share/rille/mappings`) or in the source tree during development.
fn bundled_mappings() -> Option<PathBuf> {
    let installed = std::env::current_exe().ok()?.parent()?.join("../share/rille/mappings");
    if installed.is_dir() {
        return Some(installed);
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mappings");
    dev.is_dir().then_some(dev)
}

/// `--load=A:/path/file.mp3` loads a file on start; `--sync=B` engages sync
/// and `--play=A` starts playback (demos, screenshots, quick checks);
/// `--press=deck.C.remix_cell.1` presses a control after that (once remix
/// decks have decoded their cells);
/// `--browse=<folder>` opens a folder in the browser's explorer;
/// `--size=WxH` sets the window size; `--settings` opens the settings;
/// `--scan` rescans the music folders (with `--delay=<secs>` before a
/// screenshot, a prepared profile's library gets analyzed first).
fn demo_loads(app: &std::sync::Arc<App>, args: &[String]) {
    if args.iter().any(|a| a == "--scan") {
        app.scan();
    }
    let deck_of =
        |s: &str| s.chars().next().map(|c| (c.to_ascii_uppercase() as u8).wrapping_sub(b'A')).filter(|d| *d < 4);
    let mut play = Vec::new();
    let mut sync = Vec::new();
    let mut presses: Vec<rille_core::ControlTarget> = Vec::new();
    for a in args {
        if let Some(t) = a.strip_prefix("--press=").and_then(|t| t.parse().ok()) {
            presses.push(t);
        }
        if let Some(d) = a.strip_prefix("--sync=").and_then(deck_of) {
            sync.push(d);
        }
        if let Some(spec) = a.strip_prefix("--load=")
            && let Some((d, path)) = spec.split_once(':')
            && let Some(deck) = deck_of(d)
        {
            // `--load=A:beatport:<track id>` streams a Beatport track.
            match path.strip_prefix("beatport:").and_then(|id| id.parse().ok()) {
                Some(id) => app.load_beatport(deck, id),
                None => app.load_file(deck, std::path::Path::new(path)),
            }
        }
        if let Some(d) = a.strip_prefix("--play=").and_then(deck_of) {
            play.push(d);
        }
    }
    if play.is_empty() && presses.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // Wait until the tracks are decoded (analysis may still run).
        for _ in 0..200 {
            if play.iter().all(|d| app.snapshot().decks[usize::from(*d)].loaded) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let press = |d: u8, c: rille_core::Control| {
            for down in [true, false] {
                app.control(rille_core::ControlEvent {
                    target: rille_core::ControlTarget::deck(d, c),
                    value: rille_core::ControlValue::Press(down),
                });
            }
        };
        for (i, d) in play.into_iter().enumerate() {
            if sync.contains(&d) {
                // Followers need the leader's grid: wait for its analysis.
                for _ in 0..200 {
                    if (0..4u8)
                        .all(|x| x == d || !app.snapshot().decks[usize::from(x)].playing || app.deck(x).grid.is_some())
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                press(d, rille_core::Control::Sync);
            }
            if i > 0 {
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            press(d, rille_core::Control::Play);
        }
        for _ in 0..200 {
            let decoded = |d: u8| app.deck(d).remix.is_none_or(|s| s.cells.iter().flatten().all(|c| c.audio.is_some()));
            if (0..4u8).all(decoded) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        for t in presses {
            for down in [true, false] {
                app.control(rille_core::ControlEvent { target: t, value: rille_core::ControlValue::Press(down) });
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--smoke-test" || a.starts_with("--screenshot="));
    // Headless runs use a throwaway profile unless RILLE_PROFILE (or the
    // older RILLE_PROFILE) is set.
    let throwaway = std::env::temp_dir().join(format!("rille-headless-{}", std::process::id()));
    let profile = std::env::var_os("RILLE_PROFILE").or_else(|| std::env::var_os("RILLE_PROFILE"));
    let paths = match &profile {
        Some(dir) => Paths::under(&PathBuf::from(dir)),
        None if headless => Paths::under(&throwaway),
        None => Paths::xdg(),
    };
    let mut qapp = QGuiApplication::new();
    if let Some(mut q) = qapp.as_mut() {
        q.as_mut().set_application_name(&QString::from("rille"));
        q.as_mut().set_organization_name(&QString::from("rille"));
    }
    // Wayland app id: lets the desktop match the window to rille.desktop and
    // show its icon in the dock.
    QGuiApplication::set_desktop_file_name(&QString::from("rille"));
    let opts = StartOptions { paths, audio: !headless, midi: !headless, bundled_mappings: bundled_mappings() };
    // A failed start shows the error in a window (retry, or reset the
    // library) instead of quitting without a word.
    let app = loop {
        match App::start(opts.clone()) {
            Ok(app) => break app,
            Err(e) => {
                eprintln!("rille: cannot start: {e}");
                if headless || !startup::show_error(&mut qapp, &e, &opts.paths) {
                    std::process::exit(1);
                }
            }
        }
    };
    global::set_app(app.clone());
    demo_loads(&app, &args);

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(MAIN_QML));
    }
    let code = qapp.as_mut().map_or(1, |q| q.exec());
    app.shutdown();
    if headless && profile.is_none() {
        let _ = std::fs::remove_dir_all(&throwaway);
    }
    std::process::exit(code);
}
