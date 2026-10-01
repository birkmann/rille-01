import QtQuick
import rille.ui

// The rille mark: a record seen from above (the ring) and the cue point (the
// dot at 1–2 o'clock). Geometry from the brand kit's 100-unit viewBox; small
// sizes use the kit's heavier ring and dot so the mark stays legible.
Item {
    id: mark
    property real size: 24
    property color ringColor: Theme.brandPaper
    property color dotColor: Theme.brandSignal
    implicitWidth: size
    implicitHeight: size

    readonly property real unit: size / 100
    readonly property real stroke: (size <= 24 ? 13 : size <= 48 ? 9 : 6) * unit
    readonly property real dot: (size <= 24 ? 14 : size <= 48 ? 11 : 8) * unit

    Rectangle {
        // The stroke is centred on r = 36, as in the SVG.
        x: (50 - 36) * mark.unit - mark.stroke / 2
        y: x
        width: 72 * mark.unit + mark.stroke
        height: width
        radius: width / 2
        color: "transparent"
        border.width: mark.stroke
        border.color: mark.ringColor
        antialiasing: true
    }
    Rectangle {
        x: 77.6 * mark.unit - mark.dot
        y: 26.9 * mark.unit - mark.dot
        width: mark.dot * 2
        height: width
        radius: width / 2
        color: mark.dotColor
        antialiasing: true
    }
}
