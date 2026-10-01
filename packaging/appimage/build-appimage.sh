#!/usr/bin/env bash
# Builds rille-x86_64.AppImage with linuxdeploy and its Qt plugin (downloaded
# on first use into packaging/appimage/tools).
set -euo pipefail
cd "$(dirname "$0")/../.."
here=packaging/appimage
tools="$here/tools"
mkdir -p "$tools"
fetch() {
    [[ -x "$tools/$1" ]] || { curl -L -o "$tools/$1" "$2"; chmod +x "$tools/$1"; }
}
fetch linuxdeploy-x86_64.AppImage https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch linuxdeploy-plugin-qt-x86_64.AppImage https://github.com/linuxdeploy/linuxdeploy-plugin-qt/releases/download/continuous/linuxdeploy-plugin-qt-x86_64.AppImage

appdir="$here/AppDir"
rm -rf "$appdir"
DESTDIR="$appdir" PREFIX=/usr ./scripts/install.sh
export QML_SOURCES_PATHS="$PWD/crates/rille-ui/qml"
export EXTRA_QT_MODULES="QuickControls2;QuickShapes;QuickDialogs2"
export PATH="$tools:$PATH"
linuxdeploy-x86_64.AppImage --appdir "$appdir" \
    --desktop-file packaging/rille.desktop --icon-file packaging/rille.svg \
    --plugin qt --output appimage
