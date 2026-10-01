import QtQuick
import QtQuick.Controls.Basic
import rille.ui

// Menu entry: optional icon or color swatch, text, check mark, submenu arrow.
MenuItem {
    id: item
    property string iconName: ""
    property color swatch: "transparent"
    /// Red text for destructive actions.
    property bool danger: false
    /// A non-clickable caption at the top of a menu.
    property bool caption: false
    /// Shows a check mark (without Qt's checkable toggling).
    property bool marked: false

    enabled: !caption
    implicitWidth: Math.max(220, row.implicitWidth + 50)
    implicitHeight: caption ? 26 : 30
    leftPadding: 8
    rightPadding: 8

    contentItem: Row {
        id: row
        spacing: 9
        Item {
            width: 16
            height: 16
            anchors.verticalCenter: parent.verticalCenter
            visible: !item.caption
            Icon {
                anchors.centerIn: parent
                visible: item.iconName.length > 0
                name: item.iconName
                size: 15
                color: item.danger && item.enabled ? Theme.danger : (item.highlighted ? Theme.text : (item.enabled ? Theme.textDim : Theme.textFaint))
            }
            Rectangle {
                anchors.centerIn: parent
                visible: item.swatch.a > 0
                width: 14; height: 14; radius: 3
                color: item.swatch
                border.color: Qt.lighter(item.swatch, 1.3)
            }
            Icon {
                anchors.centerIn: parent
                visible: item.marked || (item.checkable && item.checked)
                name: "check"
                size: 15
                color: Theme.sync
            }
        }
        UiText {
            anchors.verticalCenter: parent.verticalCenter
            text: item.text
            color: item.caption ? Theme.textDim : (!item.enabled ? Theme.textFaint : (item.danger ? Theme.danger : Theme.text))
            font.bold: item.caption
            font.pixelSize: item.caption ? Theme.fontSmall : Theme.fontNormal
            elide: Text.ElideRight
            width: Math.min(implicitWidth, 320)
        }
    }
    indicator: Item {}
    arrow: Icon {
        x: item.width - width - 8
        anchors.verticalCenter: parent.verticalCenter
        visible: item.subMenu !== null
        name: "chevron-right"
        size: 13
        color: Theme.textDim
    }
    background: Rectangle {
        radius: 4
        color: item.highlighted && !item.caption ? Theme.selection : "transparent"
    }
}
