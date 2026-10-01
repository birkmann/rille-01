pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import rille.ui

// Loop size selector: ‹ three sizes around the current one ›. Clicking a
// size selects it and starts the loop (or stops it, for the running size);
// the arrows step the size.
Rectangle {
    id: sel
    required property DeckController dc
    readonly property int count: dc.loopSizeCount()
    readonly property int first: Math.max(0, Math.min(count - 3, dc.loopSizeIndex - 1))

    function press(name) {
        AppController.press(dc.target(name), true)
        AppController.press(dc.target(name), false)
    }
    function choose(i) {
        var d = i - dc.loopSizeIndex
        if (d === 0) {
            press("loop_toggle")
            return
        }
        for (var k = 0; k < Math.abs(d); k++)
            press(d > 0 ? "loop_size_up" : "loop_size_down")
        if (!dc.loopActive)
            press("loop_toggle")
    }

    implicitWidth: 150
    implicitHeight: 34
    radius: Theme.radius
    color: Theme.bg
    border.color: dc.loopActive ? Theme.sync : Theme.border

    RowLayout {
        anchors.fill: parent
        anchors.margins: 2
        spacing: 1
        DjButton { flat: true; icon: "chevron-left"; target: sel.dc.target("loop_size_down"); implicitWidth: 26; tip: "Smaller loop and beat jump size"; Layout.fillHeight: true }
        Repeater {
            model: 3
            Rectangle {
                id: cell
                required property int index
                readonly property int sizeIndex: sel.first + index
                readonly property bool current: sizeIndex === sel.dc.loopSizeIndex
                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: 2
                color: current && sel.dc.loopActive ? Theme.sync : (cellArea.containsMouse ? Theme.control : "transparent")
                UiText {
                    anchors.centerIn: parent
                    text: sel.dc.loopSizeLabel(cell.sizeIndex)
                    color: cell.current ? (sel.dc.loopActive ? Theme.textOnLit : Theme.sync) : Theme.text
                    font.pixelSize: Theme.fontLarge
                    font.family: Theme.fontCondensed
                    font.bold: cell.current
                }
                MouseArea {
                    id: cellArea
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: sel.choose(cell.sizeIndex)
                }
                Tip {
                    text: cell.current && sel.dc.loopActive ? "Stop the loop" : "Loop " + sel.dc.loopSizeLabel(cell.sizeIndex) + " beats from here"
                    visible: cellArea.containsMouse && !cellArea.pressed
                }
            }
        }
        DjButton { flat: true; icon: "chevron-right"; target: sel.dc.target("loop_size_up"); implicitWidth: 26; tip: "Larger loop and beat jump size"; Layout.fillHeight: true }
    }
}
