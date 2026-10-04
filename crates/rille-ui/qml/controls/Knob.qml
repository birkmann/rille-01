import QtQuick
import QtQuick.Shapes
import rille.ui

// Rotary control: drag up/down (Shift = fine), wheel steps, double-click
// resets. A solid body with a pointer, the value ring around it, and the
// value shown while it is being turned.
Item {
    id: knob
    property real value: 0.5
    property real defaultValue: 0.5
    property string label: ""
    property string target: ""
    /// Ring drawn from the centre (EQ, filter, gain) rather than from the start.
    property bool bipolar: false
    property color color: Theme.knobArc
    property int size: 34
    /// Text for the value bubble, e.g. value => (value * 24 - 12).toFixed(1) + " dB".
    property var format: null
    /// Hover help, shown below the knob (the value bubble is above).
    property string tip: ""

    signal moved(real v)

    implicitWidth: size + 8
    implicitHeight: size + (label.length ? 15 : 2)

    function send(v) {
        v = Math.max(0, Math.min(1, v))
        if (target.length)
            AppController.setValue(target, v)
        moved(v)
    }

    Item {
        id: dial
        width: knob.size
        height: knob.size
        anchors.horizontalCenter: parent.horizontalCenter
        y: 1
        readonly property real stroke: Math.max(3, knob.size / 11)
        readonly property real ring: width / 2 - stroke / 2
        readonly property real body: width / 2 - stroke - 3

        // The geometry renderer, smoothed by multisampling: Qt 6.8's curve
        // renderer keeps a ring's first path when the knob is resized before
        // the window's first frame (the mixer columns), drawing it off-centre.
        Shape {
            anchors.fill: parent
            layer.enabled: true
            layer.samples: 4
            // Track of the ring.
            ShapePath {
                strokeColor: Theme.knobTrack
                strokeWidth: dial.stroke
                fillColor: "transparent"
                capStyle: ShapePath.FlatCap
                PathAngleArc {
                    centerX: dial.width / 2; centerY: dial.height / 2
                    radiusX: dial.ring; radiusY: dial.ring
                    startAngle: 135; sweepAngle: 270
                }
            }
            // Value.
            ShapePath {
                strokeColor: knob.color
                strokeWidth: dial.stroke
                fillColor: "transparent"
                capStyle: ShapePath.FlatCap
                PathAngleArc {
                    centerX: dial.width / 2; centerY: dial.height / 2
                    radiusX: dial.ring; radiusY: dial.ring
                    startAngle: knob.bipolar ? 270 : 135
                    sweepAngle: knob.bipolar ? (knob.value - 0.5) * 270 : knob.value * 270
                }
            }
        }
        // Centre detent mark for bipolar knobs.
        Rectangle {
            visible: knob.bipolar
            width: 2; height: 3
            x: dial.width / 2 - 1; y: -1
            color: Theme.textFaint
        }
        // Body.
        Rectangle {
            anchors.centerIn: parent
            width: dial.body * 2
            height: width
            radius: width / 2
            border.color: Theme.knobEdge
            border.width: 1
            gradient: Gradient {
                GradientStop { position: 0; color: area.containsMouse ? Theme.knobTopHover : Theme.knobTop }
                GradientStop { position: 1; color: Theme.knobBottom }
            }
            // Centre cap.
            Rectangle {
                anchors.centerIn: parent
                width: parent.width * 0.3
                height: width
                radius: width / 2
                color: Theme.knobBottom
            }
            // Pointer.
            Item {
                anchors.fill: parent
                rotation: -135 + knob.value * 270
                Rectangle {
                    width: Math.max(2.5, knob.size / 14)
                    height: parent.height * 0.38
                    radius: 1
                    x: parent.width / 2 - width / 2
                    y: 1
                    color: Theme.text
                }
            }
        }
    }
    UiText {
        visible: knob.label.length > 0
        anchors.top: dial.bottom
        anchors.topMargin: 1
        anchors.horizontalCenter: parent.horizontalCenter
        text: knob.label
        color: Theme.textDim
        font.pixelSize: Theme.fontTiny
        font.family: Theme.fontCondensed
        font.bold: true
        font.letterSpacing: 0.4
    }
    // Value bubble while turning.
    Rectangle {
        visible: area.pressed || (area.containsMouse && knob.format !== null)
        z: 10
        anchors.bottom: dial.top
        anchors.bottomMargin: 2
        anchors.horizontalCenter: dial.horizontalCenter
        width: bubble.implicitWidth + 10
        height: 18
        radius: 3
        color: Theme.tooltip
        border.color: Theme.border
        UiText {
            id: bubble
            anchors.centerIn: parent
            text: knob.format ? knob.format(knob.value) : Math.round(knob.value * 100) + "%"
            font.pixelSize: Theme.fontSmall
            font.bold: true
        }
    }
    Tip {
        text: knob.tip
        below: true
        visible: knob.tip.length > 0 && area.containsMouse && !area.pressed
    }
    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        // Inside a scrolling view (mobile mixer) the drag turns the knob.
        preventStealing: true
        property real startY: 0
        property real startValue: 0
        cursorShape: Qt.SizeVerCursor
        onPressed: mouse => {
            startY = mouse.y
            startValue = knob.value
        }
        onPositionChanged: mouse => {
            if (!pressed)
                return
            var range = (mouse.modifiers & Qt.ShiftModifier) ? 800 : 160
            knob.send(startValue + (startY - mouse.y) / range)
        }
        onDoubleClicked: knob.send(knob.defaultValue)
        onWheel: wheel => knob.send(knob.value + wheel.angleDelta.y / 120 * 0.02)
    }
}
