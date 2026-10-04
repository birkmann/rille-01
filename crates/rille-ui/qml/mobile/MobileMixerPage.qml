pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import rille.ui

// Mobile layout: the mixer view. Mixer, crossfader, master clock and both
// FX units; stacked in portrait (scrolls when the screen is short), the
// mixer beside the rest in landscape.
Flickable {
    id: page
    required property DeckController deckA
    required property DeckController deckB
    required property DeckController deckC
    required property DeckController deckD
    property bool fourDecks: false
    readonly property bool landscape: width > height

    clip: true
    contentWidth: width
    contentHeight: Math.max(height, grid.implicitHeight)
    interactive: contentHeight > height + 1
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: StyledScrollBar {}

    GridLayout {
        id: grid
        width: page.width
        height: page.contentHeight
        columns: page.landscape ? 2 : 1
        rowSpacing: Theme.gap
        columnSpacing: Theme.gap

        Mixer {
            visible: !AppController.mixerHidden
            deckA: page.deckA
            deckB: page.deckB
            deckC: page.deckC
            deckD: page.deckD
            fourDecks: page.fourDecks
            Layout.row: 0
            Layout.column: 0
            Layout.rowSpan: page.landscape ? 4 : 1
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 280
            Layout.preferredWidth: page.landscape ? 1 : -1
        }
        CrossfaderStrip {
            visible: AppController.mixerChannels.length === 0 && !AppController.mixerHidden
            compact: true
            fourDecks: page.fourDecks
            Layout.row: 1
            Layout.column: page.landscape ? 1 : 0
            Layout.fillWidth: true
            Layout.preferredWidth: page.landscape ? 1 : -1
            Layout.preferredHeight: Theme.stripHeight + 6
            Layout.fillHeight: false
        }
        MasterClockPanel {
            Layout.row: page.landscape ? 0 : 2
            Layout.column: page.landscape ? 1 : 0
            Layout.fillWidth: true
            Layout.preferredWidth: page.landscape ? 1 : -1
            Layout.preferredHeight: Theme.topRowHeight
            Layout.fillHeight: false
        }
        FxUnitPanel {
            unitIndex: 0
            Layout.row: page.landscape ? 2 : 3
            Layout.column: page.landscape ? 1 : 0
            Layout.fillWidth: true
            Layout.preferredWidth: page.landscape ? 1 : -1
            Layout.preferredHeight: Theme.topRowHeight
            Layout.fillHeight: false
        }
        FxUnitPanel {
            unitIndex: 1
            Layout.row: page.landscape ? 3 : 4
            Layout.column: page.landscape ? 1 : 0
            Layout.fillWidth: true
            Layout.preferredWidth: page.landscape ? 1 : -1
            Layout.preferredHeight: Theme.topRowHeight
            Layout.fillHeight: false
        }
    }
}
