#!/usr/bin/env bash
# Builds rille.app and rille-macos-<arch>.dmg into target/macos: the release
# binary with its Qt frameworks and QML modules (macdeployqt), the bundled
# controller mappings and the icon.
#   ./packaging/macos/build-app.sh
#   QT_ROOT_DIR=~/Qt/6.8.3/macos ./packaging/macos/build-app.sh
# Needs Rust, Qt 6.5+ and the Xcode command line tools. The app is signed ad
# hoc (no Developer ID), so a downloaded copy has to be allowed once under
# System Settings → Privacy & Security.
#
# rille.icns is packaging/rille.svg on the macOS icon grid (an 824 px tile
# with a 100 px margin on a 1024 px canvas), converted with iconutil.
set -euo pipefail
cd "$(dirname "$0")/../.."
here=packaging/macos
out=target/macos
arch=$(uname -m)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
if [[ -n "${QT_ROOT_DIR:-}" ]]; then
    export PATH="$QT_ROOT_DIR/bin:$PATH"
    export QMAKE="${QMAKE:-$QT_ROOT_DIR/bin/qmake}"
fi

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    cargo build --release -p rille-ui
fi

app="$out/rille.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/mappings"
install -m755 target/release/rille "$app/Contents/MacOS/rille"
install -m644 mappings/*.toml "$app/Contents/Resources/mappings/"
install -m644 "$here/rille.icns" LICENSE "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"

# Copies Qt, its plugins and the QML modules the UI imports into the bundle
# and points the binary at them.
macdeployqt "$app" -qmldir=crates/rille-ui/qml -verbose=1
# A Qt with every module installed (Homebrew) also gets modules the UI never
# imports, whose frameworks macdeployqt then cannot find: drop them.
contents="$app/Contents"
for m in VirtualKeyboard Pdf Scene2D Scene3D Timeline; do
    rm -rf "$contents/Resources/qml/QtQuick/$m"
done
rm -rf "$contents/PlugIns/platforminputcontexts" "$contents/PlugIns/imageformats/libqpdf.dylib" \
    "$contents"/PlugIns/quick/{libqtvkb*,libvirtualkeyboardplugin,libpdfquickplugin,libqtquickscene*,libqtquicktimeline*}.dylib
# macdeployqt rewrites the binaries, which breaks their signatures; Apple
# silicon refuses to run unsigned code, so sign the whole bundle ad hoc.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

dmg="$out/rille-macos-$arch.dmg"
stage="$out/dmg"
rm -rf "$stage" "$dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "rille $version" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg"
rm -rf "$stage"
echo "Built $app and $dmg"
