import QtQuick
import rille.ui

// One sequencer step: lit in the instrument's colour when on (full colour
// with a bar on top when accented), framed white under the playhead, dimmed
// past the pattern's end. The first step of each beat sits a little lighter.
// Clicks are handled by the row (it paints across steps).
Rectangle {
    id: step
    property int number: 1
    property bool on: false
    property bool accent: false
    property bool playhead: false
    /// Within the pattern's length.
    property bool active: true
    property bool hovered: false
    property color tint: Theme.sync
    property bool showNumber: true
    readonly property bool beatStart: (number - 1) % 4 === 0

    radius: 3
    color: on ? (accent ? tint : Qt.darker(tint, 1.45))
              : (hovered ? Theme.controlHover : (beatStart ? Theme.controlTop : Theme.controlBottom))
    border.width: playhead ? 2 : 1
    border.color: playhead ? Theme.text : (on ? Qt.lighter(tint, 1.15) : Theme.border)
    opacity: active ? 1 : 0.32

    // Accent bar.
    Rectangle {
        visible: step.on && step.accent
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 4
        height: 3
        radius: 1.5
        color: Theme.text
    }
    // The step playing now lights up.
    Rectangle {
        anchors.fill: parent
        radius: 3
        color: "white"
        opacity: step.playhead ? (step.on ? 0.4 : 0.07) : 0
    }
    UiText {
        visible: step.showNumber && parent.height >= 26
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.leftMargin: 4
        anchors.bottomMargin: 2
        text: step.number
        font.pixelSize: Theme.fontTiny
        color: step.on ? Qt.rgba(0, 0, 0, 0.55) : Theme.textFaint
    }
}
