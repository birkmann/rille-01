pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import rille.ui

// Mobile layout: the views along the bottom edge, one shown at a time.
Rectangle {
    id: bar
    property int current: 0
    /// The drum machine plays: a beat light on its tab while another is shown.
    property bool drumsPlaying: false
    property real beat: 0
    signal selected(int index)

    implicitHeight: Theme.tabBarHeight
    color: Theme.panel
    radius: Theme.radius
    border.color: Theme.panelEdge

    RowLayout {
        anchors.fill: parent
        spacing: 0
        Repeater {
            model: [
                { name: "DECKS", icon: "play" },
                { name: "MIXER", icon: "sliders" },
                { name: "DRUMS", icon: "drums" },
                { name: "LIBRARY", icon: "library" }
            ]
            Item {
                id: tab
                required property var modelData
                required property int index
                readonly property bool active: bar.current === tab.index
                objectName: "tab" + tab.index
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: 1

                Rectangle {
                    visible: tab.active
                    anchors.top: parent.top
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Math.min(parent.width - 16, 56)
                    height: 2
                    color: Theme.sync
                }
                Column {
                    anchors.centerIn: parent
                    spacing: 3
                    Icon {
                        anchors.horizontalCenter: parent.horizontalCenter
                        name: tab.modelData.icon
                        size: 20
                        color: tab.active ? Theme.text : Theme.textDim
                    }
                    UiText {
                        anchors.horizontalCenter: parent.horizontalCenter
                        text: tab.modelData.name
                        color: tab.active ? Theme.text : Theme.textDim
                        font.pixelSize: Theme.fontTiny
                        font.bold: true
                    }
                }
                Rectangle {
                    objectName: "beatLight"
                    visible: tab.index === 2 && bar.drumsPlaying && !tab.active
                    anchors.top: parent.top
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.topMargin: 6
                    anchors.horizontalCenterOffset: 16
                    width: 6
                    height: 6
                    radius: 3
                    color: Theme.play
                    opacity: ((bar.beat % 1) + 1) % 1 < 0.5 ? 1 : 0.3
                }
                MouseArea {
                    anchors.fill: parent
                    onClicked: bar.selected(tab.index)
                }
            }
        }
    }
}
