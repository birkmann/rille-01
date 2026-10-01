import QtQuick
import QtQuick.Controls.Basic
import rille.ui

// Search box with a magnifier and a clear button.
TextField {
    id: field
    implicitHeight: 28
    leftPadding: 28
    rightPadding: 26
    placeholderText: "Search"
    placeholderTextColor: Theme.textFaint
    color: Theme.text
    selectByMouse: true
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fontNormal
    background: Rectangle {
        color: Theme.bg
        radius: Theme.radius
        border.color: field.activeFocus ? Theme.sync : Theme.border
    }
    Icon {
        x: 8
        anchors.verticalCenter: parent.verticalCenter
        name: "search"
        size: 14
        color: Theme.textDim
    }
    Icon {
        id: clear
        anchors.right: parent.right
        anchors.rightMargin: 7
        anchors.verticalCenter: parent.verticalCenter
        visible: field.text.length > 0
        name: "x"
        size: 12
        color: clearArea.containsMouse ? Theme.text : Theme.textDim
        MouseArea {
            id: clearArea
            anchors.fill: parent
            anchors.margins: -4
            hoverEnabled: true
            onClicked: {
                field.clear()
                field.textEdited()
            }
        }
    }
}
