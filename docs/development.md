# Development

- [Architecture](#architecture)
- [Building and testing](#building-and-testing)
- [Accuracy](#accuracy)
- [Website and brand](#website-and-brand)

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

## Building and testing

See [Installing → From source](install.md#from-source) for the requirements.

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

## Website and brand

`website/` is the project site: static HTML and CSS, no build step, deployed
on Vercel with `website` as the root directory (`website/vercel.json`). The
brand kit (logo, colors, type, rules) is in `rille-brand/`; the app's colors
live in `crates/rille-ui/qml/Theme.qml`.
