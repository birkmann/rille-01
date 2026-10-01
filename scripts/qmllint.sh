#!/usr/bin/env bash
# Lint all QML against the module cxx-qt generated during `cargo build`.
# The generated qmldir lists files relative to itself, so assemble a temporary
# import dir that has both the qmldir/qmltypes and the crate's qml/ sources.
set -euo pipefail
cd "$(dirname "$0")/.."

profile="${1:-debug}"
gen=$(ls -td target/"$profile"/build/rille-ui-*/out/qt-build-utils/qml_modules/rille/ui 2>/dev/null | head -1)
if [[ -z "$gen" ]]; then
    echo "no generated QML module found; run 'cargo build -p rille-ui' first" >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/rille/ui"
cp "$gen/qmldir" "$gen/plugin.qmltypes" "$tmp/rille/ui/"
ln -s "$PWD/crates/rille-ui/qml" "$tmp/rille/ui/qml"

qmllint="${QMLLINT:-/usr/lib/qt6/bin/qmllint}"
shopt -s globstar
# qmllint cannot see the C++ base class (QAbstractListModel) of the list
# models defined in Rust, so those two categories are informational only.
"$qmllint" --max-warnings 0 --import info --unresolved-type info -I "$tmp" crates/rille-ui/qml/**/*.qml
