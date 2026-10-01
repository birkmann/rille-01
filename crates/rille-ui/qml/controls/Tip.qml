import QtQuick
import QtQuick.Controls.Basic
import rille.ui

// Hover help in the app's style: after a short delay a dark bubble above
// its parent (`below` for the title bar). A popup, so a clipping panel
// does not cut it off.
ToolTip {
    id: tip
    property bool below: false

    delay: 600
    margins: 6
    padding: 6
    topPadding: 4
    bottomPadding: 4
    width: Math.min(implicitWidth, 340)
    x: parent ? Math.round((parent.width - width) / 2) : 0
    y: below ? (parent ? parent.height + 4 : 0) : -height - 4

    contentItem: UiText {
        text: tip.text
        wrapMode: Text.Wrap
        color: Theme.text
        font.pixelSize: Theme.fontSmall
    }
    background: Rectangle {
        color: Theme.tooltip
        border.color: Theme.border
        radius: 3
    }
}
