pragma ComponentBehavior: Bound
import QtQuick
import rille.ui

// Linear fader bound to a control target. Double-click resets.
Item {
    id: fader
    property real value: 0.5
    property real defaultValue: 0.5
    property string target: ""
    property bool vertical: true
    /// Draw a centre notch (tempo, crossfader).
    property bool centered: false
    property color color: Theme.text
    /// Scale ticks along the track.
    property int ticks: 11
    property string tip: ""

    signal moved(real v)

    implicitWidth: vertical ? 26 : 140
    implicitHeight: vertical ? 140 : 26

    function send(v) {
        v = Math.max(0, Math.min(1, v))
        if (target.length)
            AppController.setValue(target, v)
        moved(v)
    }

    // Scale.
    Repeater {
        model: fader.ticks
        Rectangle {
            id: tick
            required property int index
            readonly property real f: tick.index / (fader.ticks - 1)
            readonly property bool major: tick.index === 0 || tick.index === fader.ticks - 1 || (fader.centered && tick.index === (fader.ticks - 1) / 2)
            color: tick.major ? Theme.textDim : Theme.textFaint
            width: fader.vertical ? (tick.major ? fader.width : fader.width - 8) : 1
            height: fader.vertical ? 1 : (tick.major ? fader.height : fader.height - 8)
            x: fader.vertical ? (fader.width - tick.width) / 2 : 5 + tick.f * (fader.width - 10)
            y: fader.vertical ? 5 + (1 - tick.f) * (fader.height - 10) : (fader.height - tick.height) / 2
            opacity: 0.6
        }
    }
    Rectangle {
        id: track
        anchors.centerIn: parent
        width: fader.vertical ? 5 : parent.width - 8
        height: fader.vertical ? parent.height - 8 : 5
        radius: 2
        color: Theme.bg
        border.color: Theme.border
    }
    // Soft halo so the cap stands out from the scale without shouting.
    Rectangle {
        x: cap.x - 2
        y: cap.y - 2
        width: cap.width + 4
        height: cap.height + 4
        radius: cap.radius + 2
        color: "transparent"
        border.width: 2
        border.color: Qt.rgba(fader.color.r, fader.color.g, fader.color.b, area.pressed ? 0.45 : 0.18)
    }
    Rectangle {
        id: cap
        width: fader.vertical ? Math.min(fader.width - 2, 28) : 16
        height: fader.vertical ? 16 : Math.min(fader.height - 2, 28)
        radius: 2
        border.color: area.pressed ? fader.color : Qt.rgba(fader.color.r, fader.color.g, fader.color.b, 0.6)
        gradient: Gradient {
            orientation: fader.vertical ? Gradient.Vertical : Gradient.Horizontal
            GradientStop { position: 0; color: Theme.knobTopHover }
            GradientStop { position: 1; color: Theme.knobTop }
        }
        x: fader.vertical ? (parent.width - width) / 2 : 4 + fader.value * (parent.width - 8 - width)
        y: fader.vertical ? 4 + (1 - fader.value) * (parent.height - 8 - height) : (parent.height - height) / 2
        Rectangle {
            anchors.centerIn: parent
            width: fader.vertical ? parent.width - 6 : 3
            height: fader.vertical ? 3 : parent.height - 6
            radius: 1
            color: fader.color
        }
    }
    Tip {
        text: fader.tip
        visible: fader.tip.length > 0 && area.containsMouse && !area.pressed
    }
    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: fader.tip.length > 0
        // Inside a scrolling view (mobile mixer) the drag moves the fader.
        preventStealing: true
        cursorShape: fader.vertical ? Qt.SizeVerCursor : Qt.SizeHorCursor
        property real grab: 0
        function valueAt(mouse) {
            if (fader.vertical)
                return 1 - (mouse.y - 4 - cap.height / 2) / (fader.height - 8 - cap.height)
            return (mouse.x - 4 - cap.width / 2) / (fader.width - 8 - cap.width)
        }
        // Grabbing the cap keeps the value (no jump); clicking the track moves there.
        onPressed: mouse => {
            var onCap = mouse.x >= cap.x && mouse.x <= cap.x + cap.width && mouse.y >= cap.y && mouse.y <= cap.y + cap.height
            grab = onCap ? valueAt(mouse) - fader.value : 0
            if (!onCap)
                fader.send(valueAt(mouse))
        }
        // hoverEnabled (for the tip) also delivers moves without a button held.
        onPositionChanged: mouse => {
            if (pressed)
                fader.send(valueAt(mouse) - grab)
        }
        onDoubleClicked: fader.send(fader.defaultValue)
        onWheel: wheel => fader.send(fader.value + wheel.angleDelta.y / 120 * 0.02)
    }
}
