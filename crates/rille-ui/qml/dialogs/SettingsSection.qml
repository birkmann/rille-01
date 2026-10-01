import QtQuick
import QtQuick.Layouts
import rille.ui

// A titled card in the settings dialog; rows go inside.
Rectangle {
    id: section
    property string title: ""
    default property alias rows: body.data

    Layout.fillWidth: true
    implicitHeight: body.implicitHeight + 44
    radius: 4
    color: Theme.panelRaised
    border.color: Theme.panelEdge

    UiText {
        x: 14
        y: 10
        text: section.title
        color: Theme.textDim
        font.pixelSize: Theme.fontSmall
        font.bold: true
        font.capitalization: Font.AllUppercase
        font.letterSpacing: 0.8
    }
    ColumnLayout {
        id: body
        x: 14
        y: 32
        width: parent.width - 28
        spacing: 10
    }
}
