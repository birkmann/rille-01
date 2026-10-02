//! The window shown when the app cannot start: the error, plus retrying or
//! starting over with an empty library.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// Why the app did not start.
        #[qproperty(QString, error)]
        /// The library database, shown before resetting it.
        #[qproperty(QString, library)]
        /// A library database exists that can be moved aside.
        #[qproperty(bool, can_reset, cxx_name = "canReset")]
        type Startup = super::StartupRust;

        /// Try starting again once the window closes.
        #[qinvokable]
        fn retry(self: &Startup);

        /// Moves the library aside and retries; returns an error text, or an
        /// empty string on success.
        #[qinvokable]
        #[cxx_name = "resetLibrary"]
        fn reset_library(self: &Startup) -> QString;
    }
}

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use rille_app::Paths;

const STARTUP_QML: &str = "qrc:/qt/qml/rille/ui/qml/StartupError.qml";

static FAILURE: Mutex<Option<(String, Paths)>> = Mutex::new(None);
static RETRY: AtomicBool = AtomicBool::new(false);

pub struct StartupRust {
    error: QString,
    library: QString,
    can_reset: bool,
}

impl Default for StartupRust {
    fn default() -> Self {
        let failure = FAILURE.lock().unwrap_or_else(|e| e.into_inner());
        let (error, db) = failure.as_ref().map(|(e, p)| (e.clone(), p.library_db())).unzip();
        Self {
            error: QString::from(error.unwrap_or_default()),
            library: QString::from(db.as_ref().map(|d| d.display().to_string()).unwrap_or_default()),
            can_reset: db.is_some_and(|d| d.exists()),
        }
    }
}

impl qobject::Startup {
    fn retry(&self) {
        RETRY.store(true, Ordering::Relaxed);
    }

    fn reset_library(&self) -> QString {
        let failure = FAILURE.lock().unwrap_or_else(|e| e.into_inner());
        let Some((_, paths)) = failure.as_ref() else { return QString::from("nothing to reset") };
        match paths.set_library_aside() {
            Ok(backup) => {
                eprintln!("rille: library moved to {}", backup.display());
                RETRY.store(true, Ordering::Relaxed);
                QString::default()
            }
            Err(e) => QString::from(format!("cannot move the library: {e}")),
        }
    }
}

/// Shows `error` until the window closes; true when the user asked to try
/// starting again.
pub fn show_error(qapp: &mut cxx::UniquePtr<QGuiApplication>, error: &str, paths: &Paths) -> bool {
    *FAILURE.lock().unwrap_or_else(|e| e.into_inner()) = Some((error.to_owned(), paths.clone()));
    RETRY.store(false, Ordering::Relaxed);
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(STARTUP_QML));
    }
    let Some(q) = qapp.as_mut() else { return false };
    q.exec();
    drop(engine);
    RETRY.load(Ordering::Relaxed)
}
