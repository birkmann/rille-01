import QtQuick
import QtQuick.Controls.Basic
import rille.ui

// Scrollbar in the app's style: always visible while there is something
// to scroll, a dark track and a wide handle that lights up when used.
ScrollBar {
    id: bar
    readonly property bool needed: bar.size < 1.0
    /// Width of the bar (height for horizontal bars).
    readonly property int thickness: 12

    policy: bar.needed ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff
    minimumSize: 0.05
    padding: 2
    implicitWidth: bar.orientation === Qt.Vertical ? bar.thickness : 100
    implicitHeight: bar.orientation === Qt.Horizontal ? bar.thickness : 100

    contentItem: Rectangle {
        implicitWidth: bar.thickness - 4
        implicitHeight: bar.thickness - 4
        radius: 4
        color: bar.pressed ? Theme.sync : (bar.hovered ? Theme.textDim : Theme.knobEdge)
    }
    background: Rectangle {
        color: Theme.bg
        radius: 5
        border.color: Theme.panelEdge
    }
}
