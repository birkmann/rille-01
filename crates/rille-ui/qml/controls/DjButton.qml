import QtQuick
import rille.ui

// Button bound to a control target: sends press on down and release on up.
// Shows text, an icon, or both.
Rectangle {
    id: btn
    property string text: ""
    property string icon: ""
    property string target: ""
    property bool lit: false
    property color litColor: Theme.accent
    property int fontSize: Theme.fontSmall
    property bool bold: true
    /// No box until hovered or lit: text buttons in the transport row.
    property bool flat: false
    /// Settings-like toggles: stay dark when on, shown by a small light and
    /// colored text instead of a filled button.
    property bool subtle: false
    /// Hover help; `tipBelow` for buttons at the top edge of the window.
    property string tip: ""
    property bool tipBelow: false
    readonly property bool filled: btn.lit && !btn.subtle

    signal clicked()
    signal rightClicked()

    implicitWidth: Math.max(32, content.implicitWidth + (btn.flat ? 12 : 18))
    implicitHeight: 28
    radius: Theme.radius
    opacity: enabled ? 1 : 0.4
    gradient: btn.flat && !btn.lit ? null : boxGradient
    color: btn.flat && !btn.lit ? (area.containsMouse ? Theme.control : "transparent") : Theme.control
    Gradient {
        id: boxGradient
        GradientStop { position: 0; color: btn.filled ? Qt.lighter(btn.litColor, 1.12) : (area.containsMouse ? Theme.controlHover : Theme.controlTop) }
        GradientStop { position: 1; color: btn.filled ? btn.litColor : (area.containsMouse ? Theme.control : Theme.controlBottom) }
    }
    border.color: btn.flat && !btn.lit ? "transparent" : (btn.filled ? Qt.lighter(btn.litColor, 1.25) : (area.pressed ? Theme.sync : Theme.border))

    // The subtle style's light.
    Rectangle {
        visible: btn.subtle
        x: 7
        anchors.verticalCenter: parent.verticalCenter
        width: 6
        height: 6
        radius: 3
        color: btn.lit ? btn.litColor : Theme.meterOff
    }

    Row {
        id: content
        anchors.centerIn: parent
        anchors.verticalCenterOffset: area.pressed ? 1 : 0
        spacing: 5
        Icon {
            visible: btn.icon.length > 0
            anchors.verticalCenter: parent.verticalCenter
            name: btn.icon
            size: 15
            color: btn.filled ? Theme.textOnLit : Theme.text
        }
        UiText {
            visible: btn.text.length > 0
            anchors.verticalCenter: parent.verticalCenter
            text: btn.text
            color: btn.filled ? Theme.textOnLit : (btn.subtle ? (btn.lit ? btn.litColor : Theme.textDim) : Theme.text)
            font.pixelSize: btn.fontSize
            font.weight: btn.bold ? Font.DemiBold : Font.Normal
            font.family: Theme.fontCondensed
            font.letterSpacing: 0.2
        }
    }
    Tip {
        text: btn.tip
        below: btn.tipBelow
        visible: btn.tip.length > 0 && area.containsMouse && !area.pressed
    }
    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        onPressed: mouse => {
            if (mouse.button === Qt.RightButton) {
                btn.rightClicked()
                return
            }
            if (btn.target.length)
                AppController.press(btn.target, true)
        }
        onReleased: mouse => {
            if (mouse.button === Qt.LeftButton && btn.target.length)
                AppController.press(btn.target, false)
        }
        onClicked: mouse => {
            if (mouse.button === Qt.LeftButton)
                btn.clicked()
        }
    }
}
