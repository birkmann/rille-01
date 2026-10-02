pragma Singleton
import QtQuick

// rille's app palette and metrics. The brand neutrals (Ink #111111,
// Paper #F4F3EF, Line #D9D7D1, Graphite #5F5D58) carried into a dark
// booth-readable UI; Signal #FF4A1C marks only the cue and the master.
// Functional colors (play, sync, FX, meters) stay distinct so a state can
// be read at a glance.
QtObject {
    readonly property color bg: "#0a0a09"
    readonly property color panel: "#151514"
    readonly property color panelRaised: "#1d1d1b"
    readonly property color panelEdge: "#23221f"
    readonly property color rowAlt: "#191917"
    readonly property color control: "#282725"
    readonly property color controlTop: "#2f2e2b"
    readonly property color controlBottom: "#232220"
    readonly property color controlHover: "#3a3935"
    readonly property color knobTop: "#3f3e3a"
    readonly property color knobTopHover: "#4a4944"
    readonly property color knobBottom: "#232220"
    readonly property color knobEdge: "#55534e"
    readonly property color knobTrack: "#3a3935"
    readonly property color knobArc: "#d9d7d1"     // Line
    readonly property color meterOff: "#272624"
    readonly property color tooltip: "#2b2a27"
    readonly property color border: "#2e2d2a"
    readonly property color text: "#f4f3ef"        // Paper
    readonly property color textDim: "#a3a19b"
    readonly property color textFaint: "#77756f"
    readonly property color accent: "#ff4a1c"      // Signal: cue, master
    readonly property color textOnLit: "#111111"    // Ink: text on lit surfaces
    readonly property color play: "#5fd068"
    readonly property color sync: "#2ec4b6"        // sync, loops
    readonly property color fx: "#8b93ff"
    readonly property color danger: "#ec3f5c"
    readonly property color warn: "#f2c94c"
    readonly property color selection: "#3d3b36"
    // Brand mark colors (see BrandMark.qml).
    readonly property color brandInk: "#111111"
    readonly property color brandPaper: "#f4f3ef"
    readonly property color brandSignal: "#ff4a1c"

    readonly property int gap: 4
    readonly property int radius: 3
    readonly property int radiusLarge: 8
    // The top part has a fixed size; the browser gets the rest of the window.
    readonly property int headerHeight: 34
    readonly property int topRowHeight: 78
    readonly property int deckRowHeight: 336
    // 4 decks: two compact decks stacked per side; the row grows with the
    // window up to this height.
    readonly property int deckRowHeight4: 520
    readonly property int stripHeight: 34
    readonly property int mixerWidth: 272
    readonly property int mixerWidth4: 296

    // Geist and Geist Mono are bundled (SIL OFL); every weight registers
    // under the same family, picked by font.weight.
    readonly property list<FontLoader> fonts: [
        FontLoader { id: geist; source: "../assets/fonts/Geist-Regular.ttf" },
        FontLoader { source: "../assets/fonts/Geist-Medium.ttf" },
        FontLoader { source: "../assets/fonts/Geist-SemiBold.ttf" },
        FontLoader { source: "../assets/fonts/Geist-Bold.ttf" },
        FontLoader { id: geistMono; source: "../assets/fonts/GeistMono-Regular.ttf" },
        FontLoader { source: "../assets/fonts/GeistMono-Medium.ttf" }
    ]
    readonly property string fontFamily: geist.status === FontLoader.Ready ? geist.font.family : "sans-serif"
    // Labels on buttons and knobs (historically a condensed face).
    readonly property string fontCondensed: fontFamily
    // Numbers that change while playing: BPM, time, tempo, clock.
    readonly property string fontMono: geistMono.status === FontLoader.Ready ? geistMono.font.family : "monospace"

    readonly property int fontTiny: 10
    readonly property int fontSmall: 11
    readonly property int fontNormal: 12
    readonly property int fontLarge: 15
    readonly property int fontHuge: 24

    // Mixer knob values as shown while turning (knob value 0..1).
    function gainText(v) {
        var db = (v - 0.5) * 24
        return (db >= 0 ? "+" : "") + db.toFixed(1) + " dB"
    }
    function filterText(v) {
        return Math.abs(v - 0.5) < 0.01 ? "OFF" : (v < 0.5 ? "LP " : "HP ") + Math.round(Math.abs(v - 0.5) * 200) + "%"
    }
    function keyShiftText(v) {
        var st = Math.round((v - 0.5) * 24)
        return (st > 0 ? "+" : "") + st + " st"
    }
    function eqText(v) {
        if (v < 0.01)
            return "KILL"
        var db = v >= 0.5 ? (v - 0.5) * 12 : 40 * Math.log(v * 2) / Math.LN10
        return (db >= 0 ? "+" : "") + db.toFixed(1) + " dB"
    }
    function eqColor(v) {
        return v < 0.02 ? danger : knobArc
    }

    function formatTime(secs) {
        var s = Math.max(0, Math.round(secs))
        var m = Math.floor(s / 60)
        var r = s % 60
        return m + ":" + (r < 10 ? "0" : "") + r
    }
}
