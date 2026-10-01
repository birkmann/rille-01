import QtQuick
import rille.ui

// On/off switch.
Item {
    id: sw
    property bool checked: false
    signal toggled()

    implicitWidth: 40
    implicitHeight: 22

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: sw.checked ? Theme.sync : Theme.control
        border.color: sw.checked ? Qt.lighter(Theme.sync, 1.2) : Theme.border
        Behavior on color { ColorAnimation { duration: 120 } }
        Rectangle {
            width: parent.height - 6
            height: width
            radius: width / 2
            y: 3
            x: sw.checked ? parent.width - width - 3 : 3
            color: sw.checked ? Theme.textOnLit : Theme.textDim
            Behavior on x { NumberAnimation { duration: 120 } }
        }
    }
    MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: {
            sw.checked = !sw.checked
            sw.toggled()
        }
    }
}
