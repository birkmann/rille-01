#!/usr/bin/env bash
# QML component tests (crates/rille-ui/tests/qml) with real mouse events.
# Components that need the Rust objects can't be loaded here, so the tests
# use a small import dir with the pure-QML pieces they need.
set -euo pipefail
cd "$(dirname "$0")/.."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/rille/ui"
q=crates/rille-ui/qml
cp "$q/Theme.qml" "$q/controls/UiText.qml" "$q/controls/Icon.qml" "$q/browser/TrackHeader.qml" \
    "$q/mobile/MobileTabBar.qml" "$tmp/rille/ui/"
# Theme.qml loads the bundled fonts relative to itself (../assets/fonts).
mkdir -p "$tmp/rille/assets"
cp -r crates/rille-ui/assets/fonts "$tmp/rille/assets/"
cat > "$tmp/rille/ui/qmldir" <<'QMLDIR'
module rille.ui
singleton Theme 1.0 Theme.qml
UiText 1.0 UiText.qml
Icon 1.0 Icon.qml
TrackHeader 1.0 TrackHeader.qml
MobileTabBar 1.0 MobileTabBar.qml
QMLDIR

runner="${QMLTESTRUNNER:-/usr/lib/qt6/bin/qmltestrunner}"
QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-offscreen}" "$runner" -import "$tmp" -input crates/rille-ui/tests/qml
