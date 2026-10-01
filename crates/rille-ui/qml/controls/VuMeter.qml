pragma ComponentBehavior: Bound
import QtQuick
import rille.ui

// Vertical peak meter with peak hold; `level` is linear (1.0 = 0 dBFS).
Item {
    id: vu
    property real level: 0
    property int segments: 24
    implicitWidth: 6
    implicitHeight: 100

    readonly property real db: level > 0 ? 20 * Math.log(level) / Math.LN10 : -60
    // −45..+3 dB mapped to the segments.
    readonly property int lit: Math.max(0, Math.min(segments, Math.round((db + 45) / 48 * segments)))
    property int peak: 0
    onLitChanged: {
        if (lit >= peak) {
            peak = lit
            peakHold.restart()
        }
    }
    Timer {
        id: peakHold
        interval: 1200
        onTriggered: vu.peak = vu.lit
    }

    Column {
        anchors.fill: parent
        spacing: 1
        Repeater {
            model: vu.segments
            Rectangle {
                required property int index
                readonly property int n: vu.segments - index
                readonly property color on: n > vu.segments - 2 ? Theme.danger : (n > vu.segments - 5 ? Theme.warn : Theme.play)
                width: vu.width
                height: (vu.height - (vu.segments - 1)) / vu.segments
                radius: 1
                color: n <= vu.lit || n === vu.peak && vu.peak > 0 ? on : Theme.meterOff
            }
        }
    }
}
