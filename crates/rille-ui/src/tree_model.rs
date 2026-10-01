//! The browser's source tree: Track Collection, Playlists, Explorer (places
//! and their folders), Music Folders and History, shown as a flat list with
//! depths. Folders are listed when expanded.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, count)]
        type BrowserTreeModel = super::BrowserTreeRust;

        #[qinvokable]
        fn refresh(self: Pin<&mut BrowserTreeModel>);

        /// Expands or collapses `row`.
        #[qinvokable]
        fn toggle(self: Pin<&mut BrowserTreeModel>, row: i32);

        /// Kind of `row`: 0 collection, 1 playlist, 2 history session,
        /// 3 folder, 4 group, 5 playlist folder, −1 none.
        #[qinvokable]
        #[cxx_name = "kindAt"]
        fn kind_at(self: &BrowserTreeModel, row: i32) -> i32;

        /// Playlist or session id, or the folder's path token.
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(self: &BrowserTreeModel, row: i32) -> i64;

        #[qinvokable]
        #[cxx_name = "labelAt"]
        fn label_at(self: &BrowserTreeModel, row: i32) -> QString;

        /// Full path of a folder row, for display.
        #[qinvokable]
        #[cxx_name = "pathTextAt"]
        fn path_text_at(self: &BrowserTreeModel, row: i32) -> QString;

        /// Whether a folder row is one of the music folders (library roots).
        #[qinvokable]
        #[cxx_name = "isMusicFolder"]
        fn is_music_folder(self: &BrowserTreeModel, row: i32) -> bool;
    }

    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut BrowserTreeModel>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut BrowserTreeModel>);

        #[cxx_override]
        fn data(self: &BrowserTreeModel, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &BrowserTreeModel) -> QHash_i32_QByteArray;
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &BrowserTreeModel, parent: &QModelIndex) -> i32;
    }
}

use core::pin::Pin;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use rille_app::explorer::{self, PlaceKind};

use crate::global::{app, path_token, token_path};

pub const KIND_COLLECTION: i32 = 0;
pub const KIND_PLAYLIST: i32 = 1;
pub const KIND_HISTORY: i32 = 2;
pub const KIND_FOLDER: i32 = 3;
pub const KIND_GROUP: i32 = 4;
pub const KIND_PLAYLIST_FOLDER: i32 = 5;

struct Node {
    label: String,
    kind: i32,
    id: i64,
    depth: i32,
    expandable: bool,
    /// Key in the expanded set.
    key: String,
    icon: &'static str,
    /// Track count or date, shown dimmed after the label.
    detail: String,
}

const ROLES: &[&str] = &["label", "kind", "nodeId", "depth", "expandable", "expanded", "icon", "detail"];

pub struct BrowserTreeRust {
    count: i32,
    nodes: Vec<Node>,
    expanded: HashSet<String>,
}

impl Default for BrowserTreeRust {
    fn default() -> Self {
        Self { count: 0, nodes: Vec::new(), expanded: ["g:playlists".to_string()].into_iter().collect() }
    }
}

fn folder_key(p: &Path) -> String {
    format!("f:{}", p.display())
}

impl BrowserTreeRust {
    fn build(&self) -> Vec<Node> {
        let Some(app) = app() else { return Vec::new() };
        let mut out = Vec::new();
        let group = |label: &str, key: &str, icon: &'static str| Node {
            label: label.into(),
            kind: KIND_GROUP,
            id: 0,
            depth: 0,
            expandable: true,
            key: key.into(),
            icon,
            detail: String::new(),
        };
        let total = app.track_count();
        out.push(Node {
            label: "Track Collection".into(),
            kind: KIND_COLLECTION,
            id: 0,
            depth: 0,
            expandable: false,
            key: "collection".into(),
            icon: "library",
            detail: total.to_string(),
        });

        out.push(group("Playlists", "g:playlists", "list"));
        if self.expanded.contains("g:playlists") {
            self.playlists(&app.playlists(), 1, &mut out);
        }

        out.push(group("Explorer", "g:explorer", "folder"));
        if self.expanded.contains("g:explorer") {
            for place in explorer::places() {
                let icon = match place.kind {
                    PlaceKind::Home => "home",
                    PlaceKind::Music => "music",
                    PlaceKind::Drive => "drive",
                    PlaceKind::Root => "computer",
                };
                self.folder(&place.path, place.name, icon, 1, &mut out);
            }
        }

        out.push(group("Music Folders", "g:roots", "music"));
        if self.expanded.contains("g:roots") {
            for root in app.settings().library_roots {
                let name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into());
                self.folder(&root, name, "folder", 1, &mut out);
            }
        }

        out.push(group("History", "g:history", "clock"));
        if self.expanded.contains("g:history") {
            for s in app.history_sessions().iter().take(50) {
                out.push(Node {
                    label: format_date(s.started),
                    kind: KIND_HISTORY,
                    id: s.id,
                    depth: 1,
                    expandable: false,
                    key: format!("h:{}", s.id),
                    icon: "clock",
                    detail: s.track_count.to_string(),
                });
            }
        }
        out
    }

    fn playlists(&self, nodes: &[rille_app::PlaylistNode], depth: i32, out: &mut Vec<Node>) {
        for n in nodes {
            let key = format!("p:{}", n.id);
            out.push(Node {
                label: n.name.clone(),
                kind: if n.is_folder { KIND_PLAYLIST_FOLDER } else { KIND_PLAYLIST },
                id: n.id,
                depth,
                expandable: n.is_folder && !n.children.is_empty(),
                key: key.clone(),
                icon: if n.is_folder { "folder" } else { "list" },
                detail: if n.is_folder { String::new() } else { n.track_count.to_string() },
            });
            if n.is_folder && self.expanded.contains(&key) {
                self.playlists(&n.children, depth + 1, out);
            }
        }
    }

    fn folder(&self, path: &Path, label: String, icon: &'static str, depth: i32, out: &mut Vec<Node>) {
        let key = folder_key(path);
        let expanded = self.expanded.contains(&key);
        out.push(Node {
            label,
            kind: KIND_FOLDER,
            id: path_token(path),
            depth,
            expandable: explorer::has_subfolders(path),
            key,
            icon,
            detail: String::new(),
        });
        if expanded && depth < 24 {
            for sub in explorer::subfolders(path) {
                let name = sub.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.folder(&sub, name, "folder", depth + 1, out);
            }
        }
    }
}

fn roles() -> QHash<QHashPair_i32_QByteArray> {
    let mut h = QHash::<QHashPair_i32_QByteArray>::default();
    for (i, n) in ROLES.iter().enumerate() {
        h.insert(256 + i as i32, QByteArray::from(*n));
    }
    h
}

impl qobject::BrowserTreeModel {
    fn refresh(mut self: Pin<&mut Self>) {
        let nodes = self.rust().build();
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().nodes = nodes;
        self.as_mut().end_reset_model();
        let n = self.rust().nodes.len() as i32;
        self.as_mut().set_count(n);
    }

    fn toggle(mut self: Pin<&mut Self>, row: i32) {
        let Some(node) = self.rust().nodes.get(row.max(0) as usize) else { return };
        if !node.expandable {
            return;
        }
        let key = node.key.clone();
        let mut r = self.as_mut().rust_mut();
        if !r.expanded.remove(&key) {
            r.expanded.insert(key);
        }
        self.refresh();
    }

    fn kind_at(&self, row: i32) -> i32 {
        self.rust().nodes.get(row.max(0) as usize).map_or(-1, |n| n.kind)
    }

    fn id_at(&self, row: i32) -> i64 {
        self.rust().nodes.get(row.max(0) as usize).map_or(-1, |n| n.id)
    }

    fn label_at(&self, row: i32) -> QString {
        QString::from(self.rust().nodes.get(row.max(0) as usize).map_or("", |n| n.label.as_str()))
    }

    fn path_text_at(&self, row: i32) -> QString {
        let p = self
            .rust()
            .nodes
            .get(row.max(0) as usize)
            .filter(|n| n.kind == KIND_FOLDER)
            .and_then(|n| token_path(n.id))
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        QString::from(p)
    }

    fn is_music_folder(&self, row: i32) -> bool {
        let Some(app) = app() else { return false };
        let path: Option<PathBuf> =
            self.rust().nodes.get(row.max(0) as usize).filter(|n| n.kind == KIND_FOLDER).and_then(|n| token_path(n.id));
        path.is_some_and(|p| app.settings().library_roots.contains(&p))
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(n) = self.rust().nodes.get(index.row().max(0) as usize) else { return QVariant::default() };
        match ROLES.get((role - 256).max(0) as usize).copied().unwrap_or("") {
            "label" => QVariant::from(&QString::from(n.label.as_str())),
            "kind" => QVariant::from(&n.kind),
            "nodeId" => QVariant::from(&n.id),
            "depth" => QVariant::from(&n.depth),
            "expandable" => QVariant::from(&n.expandable),
            "expanded" => QVariant::from(&self.rust().expanded.contains(&n.key)),
            "icon" => QVariant::from(&QString::from(n.icon)),
            "detail" => QVariant::from(&QString::from(n.detail.as_str())),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        roles()
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rust().nodes.len() as i32
    }
}

/// Unix seconds → "YYYY-MM-DD HH:MM" (UTC).
pub fn format_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs % 3600 / 60)
}
