#!/usr/bin/env bash
# Builds and installs rille. Default prefix: ~/.local (no root needed).
#   ./scripts/install.sh                 # → ~/.local
#   sudo PREFIX=/usr/local ./scripts/install.sh
set -euo pipefail
cd "$(dirname "$0")/.."
PREFIX="${PREFIX:-$HOME/.local}"
DESTDIR="${DESTDIR:-}"
export PATH="$HOME/.cargo/bin:$PATH"

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    cargo build --release -p rille-ui -p rille-cli
fi

root="$DESTDIR$PREFIX"

install -Dsm755 target/release/rille "$root/bin/rille"
install -Dsm755 target/release/rille-cli "$root/bin/rille-cli"
install -d "$root/share/rille/mappings"
install -m644 mappings/*.toml "$root/share/rille/mappings/"
install -Dm644 packaging/rille.desktop "$root/share/applications/rille.desktop"
# Launchers may not have ~/.local/bin on PATH: point at the binary directly.
if [[ -z "$DESTDIR" && "$PREFIX" != /usr ]]; then
    sed -i "s|^Exec=rille$|Exec=$PREFIX/bin/rille|" "$root/share/applications/rille.desktop"
fi
install -Dm644 packaging/rille.svg "$root/share/icons/hicolor/scalable/apps/rille.svg"
for png in packaging/icons/rille-*.png; do
    size=${png##*-}; size=${size%.png}
    install -Dm644 "$png" "$root/share/icons/hicolor/${size}x${size}/apps/rille.png"
done
install -Dm644 packaging/rille.metainfo.xml "$root/share/metainfo/io.github.birkmann.rille.metainfo.xml"
install -Dm644 LICENSE "$root/share/licenses/rille/LICENSE"
# udev only reads rules from system directories.
rules=packaging/udev/70-rille-controllers.rules
if [[ "$PREFIX" == /usr ]]; then
    install -Dm644 "$rules" "$root/lib/udev/rules.d/70-rille-controllers.rules"
fi
if [[ -z "$DESTDIR" ]]; then
    gtk-update-icon-cache -q -t "$root/share/icons/hicolor" 2>/dev/null || true
    update-desktop-database -q "$root/share/applications" 2>/dev/null || true
fi

echo "Installed to $root. Run: rille"
if [[ "$PREFIX" != /usr && ! -e /etc/udev/rules.d/70-rille-controllers.rules \
      && ! -e /usr/lib/udev/rules.d/70-rille-controllers.rules ]]; then
    echo "For Traktor Kontrol Z1 / X1 MK2 / X1 MK1 / F1 controllers, install the udev rule once:"
    echo "  sudo install -m644 $PWD/$rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger"
fi
if [[ -z "$DESTDIR" && ":$PATH:" != *":$PREFIX/bin:"* ]]; then
    echo "Note: $PREFIX/bin is not on your PATH."
fi
