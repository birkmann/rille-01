//! The app instance shared by all QML-facing objects, plus smooth playhead
//! extrapolation between engine snapshots.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use rille_app::App;
use rille_engine::{MAX_DECKS, Snapshot};

static APP: OnceLock<Arc<App>> = OnceLock::new();

pub fn set_app(app: Arc<App>) {
    let _ = APP.set(app);
}

pub fn app() -> Option<&'static Arc<App>> {
    APP.get()
}

#[derive(Clone, Copy, Default)]
struct Base {
    frames: u64,
    at: Option<Instant>,
    pos: f64,
    speed: f64,
    playing: bool,
}

static PLAYHEADS: Mutex<[Base; MAX_DECKS]> =
    Mutex::new([Base { frames: 0, at: None, pos: 0.0, speed: 0.0, playing: false }; MAX_DECKS]);

/// Position of `deck` now, interpolated between engine snapshots so the
/// waveform scrolls smoothly at any frame rate.
pub fn playhead(deck: usize, snap: &Snapshot) -> f64 {
    let d = &snap.decks[deck.min(MAX_DECKS - 1)];
    let mut heads = PLAYHEADS.lock().expect("playhead lock");
    let b = &mut heads[deck.min(MAX_DECKS - 1)];
    let now = Instant::now();
    if snap.frames != b.frames || b.at.is_none() {
        *b = Base { frames: snap.frames, at: Some(now), pos: d.position_secs, speed: d.speed, playing: d.playing };
    }
    if !b.playing {
        return d.position_secs;
    }
    let dt = b.at.map_or(0.0, |t| now.duration_since(t).as_secs_f64()).min(0.1);
    b.pos + b.speed * dt
}

// --------------------------------------------------------------- path tokens

/// Paths travel to QML as numbers: file names need not be UTF-8, and a
/// token survives any round trip through JavaScript.
#[derive(Default)]
struct PathTokens {
    by_path: std::collections::HashMap<std::path::PathBuf, i64>,
    paths: Vec<std::path::PathBuf>,
}

static TOKENS: OnceLock<Mutex<PathTokens>> = OnceLock::new();

fn tokens() -> std::sync::MutexGuard<'static, PathTokens> {
    TOKENS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// The token for `path` (the same path always gets the same token).
pub fn path_token(path: &std::path::Path) -> i64 {
    let mut t = tokens();
    if let Some(&id) = t.by_path.get(path) {
        return id;
    }
    let id = t.paths.len() as i64;
    t.paths.push(path.to_owned());
    t.by_path.insert(path.to_owned(), id);
    id
}

pub fn token_path(token: i64) -> Option<std::path::PathBuf> {
    usize::try_from(token).ok().and_then(|i| tokens().paths.get(i).cloned())
}

// ------------------------------------------------------------ changed tracks

/// Recently changed track ids with a running sequence number, so each list
/// model can update just its changed rows.
#[derive(Default)]
struct Changes {
    seq: u64,
    recent: std::collections::VecDeque<(u64, i64)>,
}

static CHANGES: OnceLock<Mutex<Changes>> = OnceLock::new();
const MAX_CHANGES: usize = 4096;

fn changes() -> std::sync::MutexGuard<'static, Changes> {
    CHANGES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn push_changed(ids: &[i64]) {
    let mut c = changes();
    for &id in ids {
        c.seq += 1;
        let seq = c.seq;
        c.recent.push_back((seq, id));
    }
    while c.recent.len() > MAX_CHANGES {
        c.recent.pop_front();
    }
}

/// Ids changed after `since`, the new sequence number, and whether some
/// changes were already dropped (then refresh everything).
pub fn changed_since(since: u64) -> (Vec<i64>, u64, bool) {
    let c = changes();
    let lost = c.recent.front().is_some_and(|(s, _)| *s > since + 1) && since < c.seq;
    let ids = c.recent.iter().filter(|(s, _)| *s > since).map(|(_, id)| *id).collect();
    (ids, c.seq, lost)
}

pub fn changes_seq() -> u64 {
    changes().seq
}
