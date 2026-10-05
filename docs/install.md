# Installing rille

- [Linux (AppImage)](#linux-appimage)
- [macOS](#macos)
- [From source](#from-source)
- [Controllers (udev rule)](#controllers-udev-rule)
- [Uninstalling](#uninstalling)

## Linux (AppImage)

Download `rille-x86_64.AppImage` from the latest GitHub release (64-bit PC,
recent distributions) and make it executable. Keeping it in `~/Applications`
is a convention, any folder works:

```sh
mkdir -p ~/Applications
mv ~/Downloads/rille-x86_64.AppImage ~/Applications/
chmod +x ~/Applications/rille-x86_64.AppImage
~/Applications/rille-x86_64.AppImage
```

In a file manager the same works with right-click → Properties → **Allow
executing file as program**. If it does not start, install `libfuse2`
(`sudo apt install libfuse2t64` on Ubuntu 24.04 and later, `libfuse2` on
older Ubuntu and Debian, `fuse2` on Arch and Fedora).

To get rille into the application menu, add a launcher and its icon:

```sh
cd ~/Applications
./rille-x86_64.AppImage --appimage-extract usr/share/icons/hicolor/scalable/apps/rille.svg
install -Dm644 squashfs-root/usr/share/icons/hicolor/scalable/apps/rille.svg \
    ~/.local/share/icons/hicolor/scalable/apps/rille.svg
rm -r squashfs-root
mkdir -p ~/.local/share/applications
cat > ~/.local/share/applications/rille.desktop <<EOF
[Desktop Entry]
Type=Application
Name=rille
Comment=DJ software
Exec=$HOME/Applications/rille-x86_64.AppImage
Icon=rille
Categories=AudioVideo;Audio;
EOF
```

To update, replace the file with the newer AppImage.

## macOS

Download `rille-macos-arm64.dmg` from the latest GitHub release (Apple
silicon, macOS 12 or later) and drag rille to Applications. The app is not
notarized: allow the first launch under System Settings → Privacy & Security
→ Open Anyway.

To build it yourself (Rust, the Xcode command line tools and Qt 6.5+, e.g.
`brew install qt`):

```sh
./packaging/macos/build-app.sh  # → target/macos/rille.app and rille-macos-arm64.dmg
```

With Qt's own installer instead of Homebrew, point `QT_ROOT_DIR` at it
(`QT_ROOT_DIR=~/Qt/6.8.3/macos`).

## From source

Requirements: Rust (stable), Qt 6.5+ (`qt6-base`, `qt6-declarative`), a C++
compiler and `clang` (for bindings), PipeWire or JACK or ALSA development files.

```sh
./scripts/install.sh            # builds and installs into ~/.local
rille
```

Other options: `packaging/arch/PKGBUILD` (`makepkg -si` in that folder),
`packaging/flatpak/io.github.birkmann.rille.yml` (KDE 6.11 runtime),
`packaging/appimage/build-appimage.sh`. The release workflow
(`.github/workflows/release.yml`) builds the Flatpak bundle, the AppImage and
the macOS disk image and attaches them to the GitHub release of every `v*`
tag; it can also be started by hand from the Actions tab. It has not run
yet, so until the first `v*` tag there are no prebuilt packages. A local
Flatpak build needs network access for its runtime and crates.

## Controllers (udev rule)

On Linux, the Traktor Kontrol controllers and the Maschine Mikro MK2 need a
udev rule so rille can open them. The Arch package installs it; otherwise
install it once:

```sh
sudo install -m644 packaging/udev/70-rille-controllers.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

See [Controllers](controllers.md) for everything else about them.

## Uninstalling

Quit rille first. Then run only the block for the way you installed it
(pacman reports `target not found: rille` when rille was not installed as a
package).

AppImage, with the launcher and icon if you added them:

```sh
rm ~/Applications/rille-x86_64.AppImage   # wherever you put it
rm -f ~/.local/share/applications/rille.desktop \
      ~/.local/share/icons/hicolor/scalable/apps/rille.svg
```

`scripts/install.sh` (use the same `PREFIX`, and `sudo`, if you changed it).
`share/rille` is also where the library lives under `~/.local`, so only its
`mappings` folder is removed here:

```sh
PREFIX=~/.local
rm -f "$PREFIX"/bin/rille "$PREFIX"/bin/rille-cli \
      "$PREFIX"/share/applications/rille.desktop \
      "$PREFIX"/share/icons/hicolor/*/apps/rille.{png,svg} \
      "$PREFIX"/share/metainfo/io.github.birkmann.rille.metainfo.xml
rm -rf "$PREFIX"/share/rille/mappings "$PREFIX"/share/licenses/rille
```

Arch package (this also removes its udev rule):

```sh
sudo pacman -R rille
```

Flatpak:

```sh
flatpak uninstall io.github.birkmann.rille
```

macOS:

```sh
rm -rf /Applications/rille.app
```

If you installed the udev rule by hand, remove it as well:

```sh
sudo rm -f /etc/udev/rules.d/70-rille-controllers.rules
sudo udevadm control --reload
```

This keeps your settings, library, analysis, cues, learned mappings, the
Beatport sign-in and the downloaded stems model, so a later install picks up
where you left off. To remove those as well (the same folders on Linux and
macOS):

```sh
rm -rf ~/.config/rille ~/.local/share/rille ~/.cache/rille
rm -rf ~/.var/app/io.github.birkmann.rille   # Flatpak keeps its data here instead
```

Recordings in your music folder's `rille recordings` are not touched.
