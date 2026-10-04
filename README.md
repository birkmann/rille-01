<p align="center">
  <img src="packaging/icons/rille-128.png" width="96" height="96" alt="">
</p>
<h1 align="center">rille</h1>
<p align="center"><b>dj software for linux and macos</b> · four decks, remix decks, beat-locked sync<br>
<a href="#installing">Install</a> · <a href="#using-it">Manual</a></p>

![rille with four decks in sync: three track decks and a remix deck](website/assets/img/rille-4-decks.png)

rille (German for the groove in a record) is a native DJ application for Linux and
macOS: four decks, remix decks, a mixer with FX, a music library and MIDI/HID
controller support, built around one priority — **beatgrids that are detected
automatically and precisely enough that sync never drifts.**

## What it does

- **Automatic beatgrids.** Every track is analyzed once (about 2 s per track):
  exact tempo, bar start, key, loudness and a spectral waveform. Grid lines
  sit on the start of the kick, measured from the average of all beats, and
  every beat is timed against that average so the tempo comes out exact
  (144.000, not 144.002). Grids are constant for produced music, piecewise
  when the tempo changes, and a per-beat map for live-played music. Only
  real doubts are flagged in the deck ("check tempo", "check bar start").
  **TICK** on a deck plays a click on every beat (higher on bar 1), so a
  grid can be checked by ear.
- **Beat-locked sync.** Synced decks follow the beat you hear from the master
  deck, not a BPM number, so the phase cannot drift — with or without keylock,
  through loops, hotcue jumps, seeks and scratches of either deck. After a
  jump the follower lands in phase at once (crossfaded), never by seconds of
  tempo pulling. Nudging a synced deck pushes it only while held.
  Half/double-time tracks sync at their natural tempo.
- **Decks:** play, CUE / CUP, 8 hotcues (cue or loop), auto loops 1/32–32
  beats, loop in/out, beatjump, flux mode, reverse, keylock (high-quality time
  stretching), key shift (a KEY knob on each mixer channel, turned on under
  Settings → Audio), tempo fader (±2–100 %), pitch bend, scratching on the
  waveform or a jog wheel, quantize and snap.
- **Stems:** **STEMS** on a deck splits its track into drums, bass, other and
  vocals, each with a level and a mute (also from a controller). The
  separation runs in the background with Meta's HTDemucs model (downloaded
  once under Settings → Decks & Analysis → Stems), takes a few minutes per
  track, and is kept for the next time the track is loaded.
- **Recording:** **REC** in the title bar records the main mix to a 24-bit
  WAV in your music folder's `rille recordings`, with a cue sheet (`.cue`)
  of the tracks played.
- **Remix decks:** any deck can be one (Settings → Decks). Four slots of 16
  sample cells each — loops and one-shots, loaded from the library or captured
  from a playing track deck's loop — started in time with the deck (quantized
  to 1/4 beat … 2 bars), with per-slot volume, filter, mute and stop. The deck
  syncs like a track deck; its cells are saved and come back on the next start.
- **Drum machine:** a 16-step sequencer with eight instruments (bass drum,
  snare, closed and open hi-hat, clap, rim shot, tom, cymbal) and 16 patterns,
  always in time and in phase with the master: step 1 falls on the leading
  track's downbeat. Three synthesized factory kits, your own kits from your
  own samples, swing, accents, live recording, per-instrument tune, decay,
  level and mute, and a channel of its own (level, filter, FX 1/2, headphone
  cue). Shown above or below the decks with **DRUMS** in the title bar; every
  control can be mapped to a controller.
- **Mixer:** gain with auto-gain (loudness levelling), 3-band isolator EQ with
  full kill, DJ filter, channel faders, crossfader, headphone cue with mix and
  volume, master limiter, meters.
- **FX:** two units with three slots each — delay, reverb, LFO filter, flanger,
  gater and beatmasher, all tempo-synced.
- **Library:** scans your music folders, reads tags and cover art, search,
  sorting, star ratings, color tags, playlists, play history, and import of a
  Traktor `collection.nml` (your corrected grids, cues, loops and ratings).
  Optional track suggestions (Settings → Library) list what fits the
  playing track by tempo, key and genre.
- **File explorer:** browse any folder, drive or USB stick in the browser,
  load files straight onto a deck, import and analyze folders (with
  subfolders) or add them as music folders. Analysis runs in the background
  with priorities (loaded decks first, then what you asked for, then the rest
  of the collection); progress, pause and cancel are in the status bar, and
  files that cannot be analyzed are not retried. A pause lasts until you
  resume, also over restarts; cancel turns the background analysis off until
  you turn it on again in Settings.
- **Controllers:** MIDI mappings (bundled: Pioneer DDJ-400, Hercules DJControl
  Inpulse 200, Allen & Heath Xone:K2 as a 4-deck controller, and a documented
  generic template), the Native Instruments Traktor Kontrol Z1, X1 MK2 and F1
  (remix decks, RGB pads, segment displays) over HID, the Maschine Mikro MK2
  for the drum machine (step sequencer and finger drumming on its pressure
  pads, its display showing the pattern), the Traktor Kontrol X1
  MK1 through the kernel's snd-usb-caiaq driver, MIDI learn, soft
  takeover, jog wheels, LED feedback, hotplug. A HID controller is described
  in its mapping file, so others can be added without changing the code.
- **Audio:** PipeWire natively, JACK, or ALSA on Linux; Core Audio on macOS.
  Headphone cue on outputs 3/4 of
  a 4-channel interface, or split mono on 2 channels. External mixer mode for
  hardware mixers such as the Allen & Heath Xone:96: every deck on its own
  mixer channel.

## Accuracy

Measured, not assumed:

| Test | Result |
|---|---|
| Synthetic tracks, 8 tempos (95–174 BPM, incl. 126.37 and 133.33) at 44.1/48/96 kHz | exact BPM, every beat within 0.37 ms, correct downbeat |
| Synthetic breakdown (16 bars without kick) / tempo change / live drift | exact / piecewise grid within 2 ms / follows the drummer within 4 ms (p95) |
| 1,142 real electronic tracks | 1,132 constant grids, 10 variable; every beat within 2 ms of the grid (p95) on 872 tracks, median p95 0.49 ms |
| Keys vs. Beatport tags (390 tracks) | 56 % exact, 72 % harmonically compatible |
| Sync on rendered audio, 60 s, 124 vs 128 BPM | worst phase error 0.016 ms (varispeed), 0.41 ms (keylock) |
| Loops | 125 wraps, every interval exact to one sample, with and without keylock |
| Sync torture test: 40 random seeks, jumps, loops, scratches, nudges, keylock and master changes per run | every follower click within 1 ms of the master's 200 ms after each operation (30 seeds) |

Run `rille-cli eval <folders>` to measure your own library, `rille-cli gridplot
<out-dir> <files>` to see every beat of a track stacked against its grid lines,
and `rille-cli click <out-dir> <files>` to write copies with a click on every
beat.

## Installing

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
tag; it can also be started by hand from the Actions tab.

### Linux (AppImage)

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

For the Traktor Kontrol and Maschine controllers, install the udev rule as described under
**Controllers** below. To update, replace the file with the newer AppImage.
To remove it, see [Uninstalling](#uninstalling).

### macOS

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

### Uninstalling

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

If you installed the udev rule for the Traktor Kontrol controllers by hand,
remove it as well:

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

## Using it

1. On first start the settings open: add your music folder under **Library**.
   Tracks are scanned and analyzed in the background.
2. Drag a track onto a deck (or double-click it, or use the **A**/**B** buttons
   that appear when hovering a row). Files can also be dropped from the file
   manager, or opened under **Explorer** in the browser. Right-click rows or
   folders for analysis, import, colors and playlists; Ctrl/Shift-click
   selects several rows. **S / M / L** next to COLUMNS switches between
   compact rows and taller rows with larger cover art.
3. Press **SYNC** on the deck you bring in: it plays in tempo and in phase with
   the master deck.

Keyboard (while not typing in the search field):

| | Deck A | Deck B |
|---|---|---|
| Play / Cue / Sync | Z / X / C | M / N / B |
| Hotcues 1–4 | 1 2 3 4 | 7 8 9 0 |
| Beatjump back / forward | Q / W | O / P |
| Loop on/off | S | K |
| Load selected track | Shift+← | Shift+→ |

**Suggestions:** switch on "Suggest tracks" under Settings → Library and
**Suggestions** appears under Track Collection in the browser. It lists the
tracks that fit the one on air (the master deck while it plays, else the deck
that has played longest), best first: the tempo has to be within 6 % (half or
double time counts), then key (Camelot neighbours) and genre decide the order.
Tracks on the decks and those played in this session are left out, and the
list follows along as the mix moves on. The **Match** column (COLUMNS) shows
the score.

**Drum machine:** **DRUMS** in the title bar shows it (Settings → Decks:
above or below the decks, one sequencer row or four). The tabs choose the
instrument the row and the TUNE, DECAY and LEVEL knobs edit; double-click a
tab (or ▸) to play it, middle-click to mute it. Click steps to switch them,
drag to paint several, right-click (or Shift-click) for an accent. Play starts
in phase with the master clock, and a pattern chosen while playing starts when
the current one comes round. With **REC** on, instruments you play while it
runs are written to the nearest step. Patterns and settings are saved as you
go, in `~/.local/share/rille/drums/state.toml`.

Your own kits live in `~/.local/share/rille/drums/kits/<kit name>/`: put
samples named after the instruments in a folder there (`BD.wav` or
`kick.wav`, `SD`/`snare`, `CH`/`hihat`, `OH`/`openhat`, `CP`/`clap`,
`RS`/`rim`, `LT`/`tom`, `CY`/`cymbal`; WAV, FLAC, MP3, OGG, AIFF), or name
the files in a `kit.toml` (`[samples]` with `BD = "my kick.wav"`). In the
drum machine's menu, "New kit from this one" copies the current kit's
sounds into a new folder; dropping a track or file on an instrument's tab
(or "Load a sound") replaces that sound, making a kit of your own first when
the current one is a factory kit. Samples longer than 8 seconds are cut.

Mapping targets (for mapping files and MIDI learn): `drum.play`,
`drum.record`, `drum.step.1`-`16` and `drum.accent.1`-`16` (the selected
instrument), `drum.cell.1`-`128` and `drum.cell_accent.1`-`128` (any
instrument: `instrument × 16 + step`), `drum.inst.1`-`8` (select),
`drum.trigger.1`-`8` (play), `drum.inst_mute/inst_level/inst_tune/inst_decay.1`-`8`,
`drum.sel_level`, `drum.sel_tune`, `drum.sel_decay` (the selected
instrument), `drum.pattern.1`-`16`, `drum.pattern_select`, `drum.inst_select`,
`drum.kit_select`, `drum.length` (relative), `drum.swing`, `drum.clear`,
`drum.clear_pattern`, `drum.level`, `drum.filter`, `drum.fx_assign.1`-`2`,
`drum.pfl`, `drum.show`, `drum.inst_solo/inst_clear.1`-`8`, `drum.repeat.1`-`8`
(held: rolls) and `drum.repeat_rate`, `drum.length_set.1`-`16`,
`drum.pattern_copy.1`-`16`, `drum.kit.1`-`16`, `drum.load_selected.1`-`8`,
`drum.nudge` (relative), `drum.undo`, `drum.redo`, `drum.copy`, `drum.paste`,
`drum.choke`, `drum.play_bar`, `drum.count_in`, `drum.replace`. Pads mapped with
`mode = "velocity"` play at the velocity of the hit, and a hard hit records or
sets an accent. For LEDs, `drum.step.N` lights the steps that are on
with the playhead running across them; `drum.step_led.N`, `drum.inst_led.N`,
`drum.trigger_led.N`, `drum.mute_led.N`, `drum.pattern_led.N`,
`drum.length_led.N`, `drum.kit_led.N` and `drum.sel_led` give RGB pad colors
and `drum.meter` the level.

**Correcting a grid:** press **GRID** on a deck. The arrows move the grid by
10/1 ms, ×2/÷2 fix half/double tempo, BEAT HERE puts a beat on the play
position, BAR START marks the first beat of a bar, GRID START does both at
once (put the play position on the first kick and press it: the grid snaps
onto the kick and its bars start there), TAP sets the tempo by tapping, the
lock protects the grid from re-analysis, RESET returns to the analyzed grid. Corrections are saved immediately.

**Controllers:** Settings → Controllers lists MIDI inputs; choose a mapping or
use **MIDI LEARN** (click a control on screen, then move the control on the
hardware). Learned mappings are saved in `~/.config/rille/mappings`. The mapping
you choose for a device is remembered for the next time it is plugged in.

A mapping that names its controller's factory MIDI channel (`channel = 15`
in the Xone:K2's file) gets a channel setting under the controller in
Settings → Controllers, for a controller set to another channel.

Some mappings come in several deck orders. The Xone:K2 drives decks C A B D
from left to right by default, so the middle columns are decks A and B;
choose "Allen & Heath Xone:K2 (ABCD)" for A B C D. A mapping file offers this
with `deck_layouts = ["CABD", "ABCD"]`, see `mappings/allen-heath-xone-k2.toml`.
For such a mapping, Settings → Controllers shows a **Decks** choice under the
controller (A B, C D, …). A controller seen for the first time takes the first
deck order no other connected controller uses, so with two X1 MK2s (or Z1s) the
second drives decks C and D. The Z1 and X1 MK2 drive decks A and B, or C and D
with their "(CD)" mapping. On the X1 MK2, SHIFT + browse press switches between
A B and C D; the choice is remembered per unit, like one made in the settings.
A mapping binds this with `target = "deck_layout:next"`. On the X1 MK2, hold
SYNC and turn the deck encoder to change that deck's tempo (a tap on SYNC
still syncs); on a synced deck this moves the tempo of every synced deck. The X1 MK2's displays
show the decks for a moment after connecting or switching, then each deck's
loop size. The X1 MK1 switches with SHIFT + HOTCUE; HOTCUE alone turns its
deck buttons into hotcues 1-8, as in Traktor.
The F1 drives remix deck C (or D, A, B with its other mappings): the pads play
the cells of the visible page (the encoder turns pages), faders and knobs are
the slot volumes and filters, the buttons below them stop the slots. Hold
SHIFT on a pad to empty it, CAPTURE to capture a loop from the source deck
into it, TYPE to switch loop/one-shot, BROWSE to load the browser's selected
track; the full table is in `mappings/traktor-kontrol-f1.toml`.

The Z1 is also a sound card. When it is connected and no other output is
chosen (or one of its own outputs is), rille plays the main mix through its MAIN
OUT and the headphone cue through its headphone jack. Its MAIN and headphone
VOL knobs are analog and act on those outputs directly.

**External mixer (Allen & Heath Xone:96):** connect USB 1 (or USB 2) and set
the mixer's channels 1-4 to that USB input. When no output device is chosen
(or any of the Xone:96's is), rille plays through the Xone:96 (it recognizes
the card by "Xone:96" in its name; under another name, choose it as the output
device and set Mixing to External), and with
Settings → Audio → Mixing on Automatic every deck goes to its own channel:
C A B D on channels 1-4 by default (outputs 1/2, 3/4, 5/6, 7/8), or A B C D
under "Mixer channels". The Xone:96 then does the faders, EQ, filters,
crossfader and headphones; rille keeps GAIN, its EQ and filter (neutral unless
you turn them), key shift and the FX units, which become inserts on the
first deck assigned to them. The on-screen fader, crossfader and headphone
controls are hidden, and each channel shows the mixer channel it feeds.
Under PipeWire switch the Xone:96's card to the "Pro Audio" profile
(pavucontrol → Configuration): its stereo profile only reaches channel 1.
The Xone:96's MIDI (channel 16 by default, sent while the MIDI 1/2 switch is lit) moves
rille's faders and crossfader, which only matters for "Dim with fader". A
Xone:K2 on its X:LINK port works through the Xone:96's USB MIDI with the
K2 mapping included in `mappings/allen-heath-xone-96.toml` (a mapping file
can pull in another with `include = ["…"]`).

The Traktor Kontrol Z1, X1 MK2 and F1 and the Maschine Mikro MK2 speak HID
rather than MIDI. rille reads them
directly and presents them as MIDI (`crates/rille-midi/src/hid.rs`), so their
mapping files, MIDI learn and soft takeover work like any other controller's.
The report layout of a HID controller is the `[hid]` table of its mapping file
(byte offsets of knobs, buttons, encoders, touch strip, LEDs, RGB pads,
pressure pads, segment and pixel displays); a mapping with such a table makes
rille look for that device, so a new HID controller needs a mapping file, not a
new build. See the files in `mappings/` and the field list in `hid.rs`.

The Traktor Kontrol X1 MK1 is not HID: the kernel's snd-usb-caiaq driver
claims it. rille reads the driver's input device, rebuilds its events into
reports for the same kind of `[hid]` table (`transport = "caiaq"`, see
`crates/rille-midi/src/caiaq.rs`) and sets the LEDs through the mixer
controls of the driver's sound card.
Opening them needs a udev rule, which the Arch package installs; otherwise
install it once (a new HID controller needs its own line in the rule):

    sudo install -m644 packaging/udev/70-rille-controllers.rules /etc/udev/rules.d/
    sudo udevadm control --reload && sudo udevadm trigger

**Maschine Mikro MK2:** plugged in, it drives the drum machine and opens it;
it works next to any DJ controller. The pads, button lights, the RGB pads and
the display are mapped. The display is drawn by rille: pattern, transport, the
selected instrument, tempo, all eight instruments' steps with the playhead and
what the encoder edits; while a button is held it shows what the pads do.

![The Mikro's display as rille draws it: the steps with the playhead, and while PATTERN, MUTE or NOTE REPEAT is held](website/assets/img/mikro-screens.png)

Steps, patterns, lengths and kits count from the top-left pad, row by row; the
instruments are the top two rows, A–H as printed (BD SD CH OH / CP RS LT CY).

| On the Mikro | Does |
|---|---|
| Pads | Steps 1–16 of the selected instrument; a hard hit sets an accent |
| PAD MODE | Top two rows play the instruments with velocity (recorded while REC is on), bottom two rows mute them; again: back to steps (also SHIFT+GROUP, STEP MODE) |
| GROUP or SELECT + pad | Select the instrument |
| MUTE / SOLO + pad | Mute / solo the instrument; SHIFT+MUTE (CHOKE): closed hi-hat cuts the open one |
| ERASE + pad | Clear the instrument's steps; SHIFT+ERASE clears the pattern |
| PATTERN + pad | Pattern 1–16, from the next bar while playing (tap PATTERN to keep it up) |
| DUPLICATE + pad | Copy the pattern there |
| GRID + pad | Pattern length 1–16 |
| SCENE + pad | Kit 1–16 (BROWSE + encoder: next or previous kit) |
| NOTE REPEAT + pad | Rolls in time while held; the encoder sets 1/4 … 1/32 (tap NOTE REPEAT to keep it on) |
| SAMPLING + pad | The track selected in the library as the instrument's sound; a tap on SAMPLING: drums in the headphones |
| SHIFT + pad | As printed: UNDO, REDO, QUANTIZE (no swing), QUANT 50% (half swing), NUDGE ◄ ►, CLEAR, CLR AUTO (the pattern), COPY, PASTE, SEMITONE − +, OCTAVE − + |
| PLAY · RESTART | Start/stop; start on the next downbeat or stop at the end of the bar |
| REC | Live recording; SHIFT+REC (COUNT-IN) from the next downbeat; ERASE+REC (REPLACE) a new take clears the instrument's old steps |
| ◄ STEP ► · ◄ ► | Previous/next pattern · previous/next instrument |
| Encoder | F1 level, F2 tune, F3 decay (the selected instrument), CONTROL filter, MAIN volume; SHIFT+turn swing; push: back to the default |
| VIEW · NAV | Show/hide the drum machine · drums through FX 1 (SHIFT: FX 2) |

The mapping file is generated by `scripts/gen-maschine-mikro-mk2.py`; change
the script and run it again rather than editing the file.

**Beatport streaming:** sign in under Settings → Beatport (needs a Beatport
streaming subscription; lossless FLAC needs Professional). The browser's
**Beatport** section then searches the catalog from the search box (a pasted
beatport.com link to a track, release, chart, playlist, label, artist or genre
lists its tracks), shows your Beatport playlists and the tracks you streamed
before. A loaded track starts playing once its first seconds have arrived and
keeps downloading while it plays (the deck shows how far; should playback
catch up, the deck waits with "Buffering…"). The file is kept in
`~/.cache/rille/beatport`, up to the cache size set in the
settings; cues, beatgrid edits and analysis are kept like for any other track,
also when the file has to be downloaded again. rille keeps the sign-in tokens
in `~/.local/share/rille/beatport-token.json`, never the password.

Files: settings in `~/.config/rille`, the library in `~/.local/share/rille`,
cover thumbnails in `~/.cache/rille`. Set `RILLE_PROFILE=<dir>` to keep
everything in one folder instead (portable setups, tests).

## Architecture

| Crate | Purpose |
|---|---|
| `rille-core` | Shared types: beatgrids (constant / piecewise / live), quantize math, keys, control IDs, cues |
| `rille-decode` | Decoding for playback and analysis alike, so positions match exactly |
| `rille-analysis` | Tempo, lattice fit, transient refinement, grid classification, downbeat, key, loudness, waveform |
| `rille-dsp` | Real-time DSP: EQ, filter, limiter, meters, resampling, the six effects |
| `rille-engine` | Audio engine: decks, beat-locked sync, keylock, mixer, cpal backends |
| `rille-library` | SQLite collection, scanning, tags, covers, playlists, history, NML import |
| `rille-midi` | MIDI mappings, learn, soft takeover, jog, LED feedback, devices |
| `rille-beatport` | Beatport sign-in, catalog search and lists, track downloads (readable while they download) |
| `rille-stems` | Stem separation with HTDemucs on the CPU (tract) |
| `rille-app` | Application core without UI: loading, background analysis, persistence |
| `rille-ui` | Qt Quick interface and the `rille` binary |
| `rille-cli` | The `rille-cli` developer tool: `analyze`, `eval`, `gridplot`, `click`, `play`, `chroma-dump`, `downbeat-dump` |

The audio thread never allocates, locks or waits: commands arrive through a
lock-free queue, state goes out through a triple buffer, and memory it no longer
needs is freed on the app side.

## Development

```sh
cargo test --workspace --release    # all tests incl. the beatgrid accuracy suite
cargo clippy --workspace --all-targets -- -D warnings
./scripts/qmllint.sh release        # after building rille-ui
QT_QPA_PLATFORM=offscreen QT_FATAL_WARNINGS=1 ./target/debug/rille --smoke-test
QT_QPA_PLATFORM=offscreen ./target/release/rille --screenshot=/tmp/rille.png --load=A:song.mp3 --play=A
QT_QPA_PLATFORM=offscreen ./target/release/rille --screenshot=/tmp/rille.png --browse=~/Music   # explorer
RILLE_PROFILE=<dir> QT_QPA_PLATFORM=offscreen ./target/release/rille --scan --delay=60 --screenshot=/tmp/rille.png   # scan + analyze a prepared library first
RILLE_TORTURE_SEEDS=30 cargo test --release -p rille-engine --test sync_torture              # long sync test
rille-cli analyze song.mp3             # grid report for one file
rille-cli play song.mp3 10             # engine on the real audio device
RILLE_TIMING=1 RILLE_DEBUG_OBS=1 rille-cli analyze song.mp3   # analysis internals
RILLE_DEBUG_DOWNBEAT=1 RILLE_DEBUG_BARS=1 rille-cli analyze song.mp3   # bar start evidence
rille-cli beatport login <user>        # then: search <text>, list <url>, playlists, fetch <id> <dir>
RILLE_STEM_MODEL=htdemucs.onnx cargo test --release -p rille-stems -p rille-app -- --ignored   # stems with the real model
```

## Limitations

- Stems need the HTDemucs model, downloaded once (316 MB; Meta released its
  weights for research use only, so they are not part of rille), and a few
  minutes of CPU time per track. There is no GPU path.
- The bar start comes from the track's section changes (drops, breakdowns),
  helped by a model of what bar 1 sounds like (fitted on tracks whose section
  changes agree). On loop-based music without section changes it can still
  be off by a beat. Such tracks are marked "check bar start"; fix them with
  BAR START in the grid editor.
- The bundled DDJ-400, Inpulse 200, Xone:K2, Z1, X1 MK2, X1 MK1, F1 and AMX mappings
  were converted from community mappings and have not all been
  tested on the hardware; the tempo fader direction may need `invert = true`.
  The X1 MK2's display layout comes from a community script and has not been
  checked on a device. The X1 MK1's encoder direction and SHIFT/HOTCUE LEDs
  come from the kernel driver and a community tool and have not been checked
  on a device either. Xone:96 support follows its user guide and has not
  been tested on the hardware either. Other HID controllers need a mapping
  file with their report layout.
- The Flatpak, AppImage and macOS disk image are built by the release
  workflow on GitHub, which has not run yet; until the first `v*` tag there
  are no prebuilt packages. The macOS app is signed ad hoc, not notarized,
  and built for Apple silicon only. A local Flatpak build needs network access for its runtime and
  crates.
- Recording takes the internal main mix; with external mixing, record on the
  hardware mixer. A recording stops when the audio output changes.
- Beatport: AAC files whose index sits at the end of the file only play once
  that part has arrived. The sign-in uses Beatport's own web client, as
  beatportdl does, not an official partner integration.

## Website and brand

`website/` is the project site: static HTML and CSS, no build step, deployed
on Vercel with `website` as the root directory (`website/vercel.json`). The brand kit (logo,
colors, type, rules) is in `rille-brand/`; the app's colors live in
`crates/rille-ui/qml/Theme.qml`.

## Credits

Most bundled controller mappings and the HID report layouts are derived from
existing community mappings (GPL-2.0-or-later); each file in `mappings/` names
its sources and authors.

## License

Copyright (C) 2026 The rille contributors

GPL-3.0-or-later. See `LICENSE`. The bundled Geist and Geist Mono fonts are
under the SIL Open Font License (`crates/rille-ui/assets/fonts/OFL.txt`).

rille is not affiliated with or endorsed by Native Instruments, Pioneer DJ,
Allen & Heath, Hercules, Akai Professional or Beatport.
Product names are trademarks of their respective owners and are used only to
identify compatible hardware and file formats.
