import QtQuick
import QtQuick.Layouts
import rille.ui

// The strip between decks and browser: the crossfader exactly under the
// mixer (same column widths as the deck row), the headphone mix and
// volume at the right end.
Rectangle {
    id: strip
    property int mixerWidth: Theme.mixerWidth
    /// Decks A and C play on the left side, B and D on the right.
    property bool fourDecks: false
    /// Mobile layout: the crossfader takes the free width, no mixer column.
    property bool compact: false
    color: Theme.panel
    radius: Theme.radius
    border.color: Theme.panelEdge

    RowLayout {
        anchors.fill: parent
        spacing: Theme.gap

        Item { visible: !strip.compact; Layout.fillWidth: true; Layout.preferredWidth: 1 }

        RowLayout {
            Layout.preferredWidth: strip.compact ? -1 : strip.mixerWidth
            Layout.fillWidth: strip.compact
            Layout.fillHeight: true
            spacing: 6
            UiText { Layout.leftMargin: 8; text: strip.fourDecks ? "A C" : "A"; color: Theme.textDim; font.bold: true; font.family: Theme.fontCondensed; font.pixelSize: Theme.fontLarge }
            Fader {
                Layout.fillWidth: true
                Layout.fillHeight: true
                vertical: false
                centered: true
                ticks: 9
                value: AppController.crossfader
                target: "global.crossfader"
                tip: "Crossfader; double-click centres it"
            }
            UiText { Layout.rightMargin: 8; text: strip.fourDecks ? "B D" : "B"; color: Theme.textDim; font.bold: true; font.family: Theme.fontCondensed; font.pixelSize: Theme.fontLarge }
        }

        RowLayout {
            Layout.fillWidth: !strip.compact
            Layout.preferredWidth: strip.compact ? -1 : 1
            Layout.fillHeight: true
            spacing: 6
            Item { visible: !strip.compact; Layout.fillWidth: true }
            Icon { name: "headphones"; size: 16; color: Theme.textDim }
            Knob {
                size: 24
                value: AppController.cueMix
                target: "global.cue_mix"
                bipolar: true
                tip: "Headphone mix: left = cued channels only, right = main mix only"
                format: v => v < 0.02 ? "CUE" : (v > 0.98 ? "MAIN" : Math.round(v * 100) + "% main")
            }
            UiText { text: "MIX"; color: Theme.textDim; font.pixelSize: Theme.fontTiny; font.bold: true }
            Knob { size: 24; value: AppController.cueVolume; defaultValue: 0.8; target: "global.cue_volume"; color: Theme.warn; tip: "Headphone volume" }
            UiText { Layout.rightMargin: 10; text: "VOL"; color: Theme.textDim; font.pixelSize: Theme.fontTiny; font.bold: true }
        }
    }
}
