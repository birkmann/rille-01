# Manual

- [Getting started](#getting-started)
- [Keyboard](#keyboard)
- [Suggestions](#suggestions)
- [Correcting a grid](#correcting-a-grid)
- [Beatport streaming](#beatport-streaming)
- [Where rille keeps its files](#where-rille-keeps-its-files)

The drum machine and controllers have pages of their own:
[Drum machine](drum-machine.md) and [Controllers](controllers.md).

## Getting started

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

## Keyboard

While not typing in the search field:

| | Deck A | Deck B |
|---|---|---|
| Play / Cue / Sync | Z / X / C | M / N / B |
| Hotcues 1–4 | 1 2 3 4 | 7 8 9 0 |
| Beatjump back / forward | Q / W | O / P |
| Loop on/off | S | K |
| Load selected track | Shift+← | Shift+→ |

## Suggestions

Switch on "Suggest tracks" under Settings → Library and **Suggestions**
appears under Track Collection in the browser. It lists the tracks that fit
the one on air (the master deck while it plays, else the deck that has played
longest), best first: the tempo has to be within 6 % (half or double time
counts), then key (Camelot neighbours) and genre decide the order. Tracks on
the decks and those played in this session are left out, and the list follows
along as the mix moves on. The **Match** column (COLUMNS) shows the score.

## Correcting a grid

Press **GRID** on a deck. The arrows move the grid by 10/1 ms, ×2/÷2 fix
half/double tempo, BEAT HERE puts a beat on the play position, BAR START marks
the first beat of a bar, GRID START does both at once (put the play position
on the first kick and press it: the grid snaps onto the kick and its bars
start there), TAP sets the tempo by tapping, the lock protects the grid from
re-analysis, RESET returns to the analyzed grid. Corrections are saved
immediately.

The bar start comes from the track's section changes (drops, breakdowns),
helped by a model of what bar 1 sounds like (fitted on tracks whose section
changes agree). On loop-based music without section changes it can still be
off by a beat. Such tracks are marked "check bar start"; fix them with BAR
START.

## Beatport streaming

Sign in under Settings → Beatport (needs a Beatport streaming subscription;
lossless FLAC needs Professional). The browser's **Beatport** section then
searches the catalog from the search box (a pasted beatport.com link to a
track, release, chart, playlist, label, artist or genre lists its tracks),
shows your Beatport playlists and the tracks you streamed before.

A loaded track starts playing once its first seconds have arrived and keeps
downloading while it plays (the deck shows how far; should playback catch up,
the deck waits with "Buffering…"). The file is kept in
`~/.cache/rille/beatport`, up to the cache size set in the settings; cues,
beatgrid edits and analysis are kept like for any other track, also when the
file has to be downloaded again. rille keeps the sign-in tokens in
`~/.local/share/rille/beatport-token.json`, never the password.

## Where rille keeps its files

Settings in `~/.config/rille`, the library in `~/.local/share/rille`, cover
thumbnails in `~/.cache/rille`. Set `RILLE_PROFILE=<dir>` to keep everything
in one folder instead (portable setups, tests).
