# Controllers

- [Choosing a mapping and MIDI learn](#choosing-a-mapping-and-midi-learn)
- [Deck orders](#deck-orders)
- [Traktor Kontrol X1, F1 and Z1](#traktor-kontrol-x1-f1-and-z1)
- [External mixer (Allen & Heath Xone:96)](#external-mixer-allen--heath-xone96)
- [HID controllers](#hid-controllers)
- [Not tested on the hardware yet](#not-tested-on-the-hardware-yet)

The Maschine Mikro MK2 drives the drum machine; it is described on the
[Drum machine](drum-machine.md#maschine-mikro-mk2) page, with the drum
machine's mapping targets.

## Choosing a mapping and MIDI learn

Settings → Controllers lists MIDI inputs; choose a mapping or use **MIDI
LEARN** (click a control on screen, then move the control on the hardware).
Learned mappings are saved in `~/.config/rille/mappings`. The mapping you
choose for a device is remembered for the next time it is plugged in.

A mapping that names its controller's factory MIDI channel (`channel = 15` in
the Xone:K2's file) gets a channel setting under the controller in
Settings → Controllers, for a controller set to another channel.

## Deck orders

Some mappings come in several deck orders. The Xone:K2 drives decks C A B D
from left to right by default, so the middle columns are decks A and B;
choose "Allen & Heath Xone:K2 (ABCD)" for A B C D. A mapping file offers this
with `deck_layouts = ["CABD", "ABCD"]`, see `mappings/allen-heath-xone-k2.toml`.

For such a mapping, Settings → Controllers shows a **Decks** choice under the
controller (A B, C D, …). A controller seen for the first time takes the first
deck order no other connected controller uses, so with two X1 MK2s (or Z1s)
the second drives decks C and D. A mapping binds switching with
`target = "deck_layout:next"`.

## Traktor Kontrol X1, F1 and Z1

- **X1 MK2:** drives decks A and B, or C and D with its "(CD)" mapping.
  SHIFT + browse press switches between A B and C D; the choice is remembered
  per unit, like one made in the settings. Hold SYNC and turn the deck encoder
  to change that deck's tempo (a tap on SYNC still syncs); on a synced deck
  this moves the tempo of every synced deck. The displays show the decks for a
  moment after connecting or switching, then each deck's loop size.
- **X1 MK1:** switches decks with SHIFT + HOTCUE; HOTCUE alone turns its deck
  buttons into hotcues 1-8, as in Traktor.
- **F1:** drives remix deck C (or D, A, B with its other mappings). The pads
  play the cells of the visible page (the encoder turns pages), faders and
  knobs are the slot volumes and filters, the buttons below them stop the
  slots. Hold SHIFT on a pad to empty it, CAPTURE to capture a loop from the
  source deck into it, TYPE to switch loop/one-shot, BROWSE to load the
  browser's selected track; the full table is in
  `mappings/traktor-kontrol-f1.toml`.
- **Z1:** drives decks A and B, or C and D with its "(CD)" mapping. It is also
  a sound card: when it is connected and no other output is chosen (or one of
  its own outputs is), rille plays the main mix through its MAIN OUT and the
  headphone cue through its headphone jack. Its MAIN and headphone VOL knobs
  are analog and act on those outputs directly.

## External mixer (Allen & Heath Xone:96)

Connect USB 1 (or USB 2) and set the mixer's channels 1-4 to that USB input.
When no output device is chosen (or any of the Xone:96's is), rille plays
through the Xone:96 (it recognizes the card by "Xone:96" in its name; under
another name, choose it as the output device and set Mixing to External), and
with Settings → Audio → Mixing on Automatic every deck goes to its own
channel: C A B D on channels 1-4 by default (outputs 1/2, 3/4, 5/6, 7/8), or
A B C D under "Mixer channels".

The Xone:96 then does the faders, EQ, filters, crossfader and headphones;
rille keeps GAIN, its EQ and filter (neutral unless you turn them), key shift
and the FX units, which become inserts on the first deck assigned to them. The
on-screen fader, crossfader and headphone controls are hidden, and each
channel shows the mixer channel it feeds.

Under PipeWire switch the Xone:96's card to the "Pro Audio" profile
(pavucontrol → Configuration): its stereo profile only reaches channel 1.

The Xone:96's MIDI (channel 16 by default, sent while the MIDI 1/2 switch is
lit) moves rille's faders and crossfader, which only matters for "Dim with
fader". A Xone:K2 on its X:LINK port works through the Xone:96's USB MIDI with
the K2 mapping included in `mappings/allen-heath-xone-96.toml` (a mapping file
can pull in another with `include = ["…"]`).

## HID controllers

The Traktor Kontrol Z1, X1 MK2 and F1 and the Maschine Mikro MK2 speak HID
rather than MIDI. rille reads them directly and presents them as MIDI
(`crates/rille-midi/src/hid.rs`), so their mapping files, MIDI learn and soft
takeover work like any other controller's.

The report layout of a HID controller is the `[hid]` table of its mapping file
(byte offsets of knobs, buttons, encoders, touch strip, LEDs, RGB pads,
pressure pads, segment and pixel displays); a mapping with such a table makes
rille look for that device, so a new HID controller needs a mapping file, not
a new build. See the files in `mappings/` and the field list in `hid.rs`.

The Traktor Kontrol X1 MK1 is not HID: the kernel's snd-usb-caiaq driver
claims it. rille reads the driver's input device, rebuilds its events into
reports for the same kind of `[hid]` table (`transport = "caiaq"`, see
`crates/rille-midi/src/caiaq.rs`) and sets the LEDs through the mixer controls
of the driver's sound card.

On Linux, opening these controllers needs the
[udev rule](install.md#controllers-udev-rule); a new HID controller needs its
own line in it.

## Not tested on the hardware yet

The bundled DDJ-400, Inpulse 200, Xone:K2, Z1, X1 MK2, X1 MK1, F1 and AMX
mappings were converted from community mappings and have not all been tested
on the hardware; the tempo fader direction may need `invert = true`. The X1
MK2's display layout comes from a community script and has not been checked on
a device. The X1 MK1's encoder direction and SHIFT/HOTCUE LEDs come from the
kernel driver and a community tool and have not been checked on a device
either. Xone:96 support follows its user guide and has not been tested on the
hardware either.
