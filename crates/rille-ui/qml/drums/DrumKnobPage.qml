pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import rille.ui

// Tabs over a row of three knobs, for a section with more settings than
// room. `pages` lists {name, tip, knobs}; each knob is {label, target,
// value, defaultValue, bipolar, format, tip}. A page with fewer knobs keeps
// the row's width. A tab whose knobs are away from their defaults shows a
// dot, so effects left on are easy to find.
ColumnLayout {
    id: kp
    property var pages: []
    property int current: 0
    property color color: Theme.text
    property int knobSize: 30
    readonly property var page: kp.pages[Math.min(kp.current, kp.pages.length - 1)] || ({ knobs: [] })

    function changed(p) {
        return !!p && p.knobs.some(k => Math.abs(k.value - k.defaultValue) > 0.001)
    }

    spacing: 2
    RowLayout {
        Layout.fillWidth: true
        spacing: 3
        // Counts, not the pages themselves, so the buttons and knobs stay
        // put (and a knob keeps its drag) while the values change.
        Repeater {
            model: kp.pages.length
            DjButton {
                id: tabButton
                required property int index
                readonly property var p: kp.pages[index]
                Layout.fillWidth: true
                Layout.preferredWidth: 30
                implicitHeight: 16
                text: p ? p.name : ""
                fontSize: Theme.fontTiny
                lit: kp.current === index
                litColor: kp.color
                tip: p ? p.tip : ""
                onClicked: kp.current = index
                Rectangle {
                    visible: !tabButton.lit && kp.changed(tabButton.p)
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.margins: 2
                    width: 4
                    height: 4
                    radius: 2
                    color: kp.color
                }
            }
        }
    }
    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        spacing: 0
        Repeater {
            model: 3
            Knob {
                required property int index
                readonly property var k: kp.page.knobs[index]
                opacity: k ? 1 : 0
                enabled: !!k
                size: kp.knobSize
                color: kp.color
                label: k ? k.label : ""
                target: k ? k.target : ""
                value: k ? k.value : 0
                defaultValue: k ? k.defaultValue : 0
                bipolar: k ? !!k.bipolar : false
                format: k && k.format ? k.format : null
                tip: k && k.tip ? k.tip : ""
            }
        }
    }
}
