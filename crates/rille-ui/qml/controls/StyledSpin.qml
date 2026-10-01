import QtQuick
import QtQuick.Layouts
import rille.ui

// Number stepper: − value +. Wheel and hold-free clicks step by one.
Rectangle {
    id: spin
    property int value: 0
    property int from: 0
    property int to: 100
    property string suffix: ""
    signal valueModified()

    function step(d) {
        var v = Math.max(from, Math.min(to, value + d))
        if (v !== value) {
            value = v
            valueModified()
        }
    }

    implicitWidth: 118
    implicitHeight: 28
    radius: Theme.radius
    color: Theme.bg
    border.color: Theme.border

    RowLayout {
        anchors.fill: parent
        anchors.margins: 2
        spacing: 0
        DjButton { flat: true; icon: "minus"; implicitWidth: 26; Layout.fillHeight: true; onClicked: spin.step(-1) }
        UiText {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: spin.value + (spin.suffix.length ? " " + spin.suffix : "")
            font.bold: true
        }
        DjButton { flat: true; icon: "plus"; implicitWidth: 26; Layout.fillHeight: true; onClicked: spin.step(1) }
    }
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.NoButton
        onWheel: wheel => spin.step(wheel.angleDelta.y > 0 ? 1 : -1)
    }
}
