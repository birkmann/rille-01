import QtQuick
import QtQuick.Layouts
import rille.ui

// One setting: name and a short explanation on the left, the control on the right.
RowLayout {
    id: row
    property string label: ""
    property string hint: ""
    default property alias control: slot.data

    // Always as wide as the card, so controls line up on the right even
    // for rows without an explanation.
    Layout.fillWidth: true
    Layout.preferredWidth: 4000
    spacing: 16

    ColumnLayout {
        Layout.fillWidth: true
        spacing: 2
        UiText { text: row.label; font.bold: true }
        UiText {
            visible: row.hint.length > 0
            Layout.fillWidth: true
            text: row.hint
            color: Theme.textDim
            font.pixelSize: Theme.fontSmall
            wrapMode: Text.WordWrap
        }
    }
    Item {
        id: slot
        Layout.alignment: Qt.AlignVCenter | Qt.AlignRight
        implicitWidth: childrenRect.width
        implicitHeight: childrenRect.height
    }
}
