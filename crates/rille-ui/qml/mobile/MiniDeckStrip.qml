pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import rille.ui

// Mobile layout, on the views without decks: play/pause, title and time
// left of each deck. Tapping a deck goes back to the decks.
Rectangle {
    id: strip
    required property list<DeckController> decks
    signal deckRequested(int deck)

    implicitHeight: Theme.miniStripHeight
    color: Theme.panel
    radius: Theme.radius
    border.color: Theme.panelEdge

    RowLayout {
        anchors.fill: parent
        anchors.margins: 4
        spacing: Theme.gap
        Repeater {
            model: strip.decks
            Rectangle {
                id: cell
                required property DeckController modelData
                readonly property DeckController dc: cell.modelData
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: 1
                color: Theme.panelRaised
                radius: Theme.radius
                clip: true

                TapHandler {
                    onTapped: strip.deckRequested(cell.dc.deck)
                }
                RowLayout {
                    anchors.fill: parent
                    anchors.rightMargin: 6
                    spacing: 6
                    DjButton {
                        icon: cell.dc.playing ? "pause" : "play"
                        text: String.fromCharCode(65 + cell.dc.deck)
                        target: cell.dc.target("play")
                        lit: true
                        litColor: cell.dc.playing ? Theme.play : Qt.darker(Theme.play, 1.8)
                        implicitWidth: 52
                        Layout.fillHeight: true
                    }
                    UiText {
                        visible: strip.decks.length <= 2
                        Layout.fillWidth: true
                        text: cell.dc.loaded || cell.dc.loading ? cell.dc.title : "Empty"
                        color: cell.dc.loaded ? Theme.text : Theme.textFaint
                        font.pixelSize: Theme.fontSmall
                        elide: Text.ElideRight
                    }
                    Item { visible: strip.decks.length > 2; Layout.fillWidth: true }
                    UiText {
                        text: cell.dc.loaded ? "−" + Theme.formatTime(cell.dc.duration - cell.dc.position) : "—"
                        color: cell.dc.endWarning ? Theme.danger : Theme.textDim
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.fontSmall
                    }
                }
                // Playback position.
                Rectangle {
                    anchors.bottom: parent.bottom
                    height: 2
                    width: cell.dc.duration > 0 ? parent.width * Math.min(1, cell.dc.position / cell.dc.duration) : 0
                    color: cell.dc.endWarning ? Theme.danger : Theme.sync
                }
            }
        }
    }
}
