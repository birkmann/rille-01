pragma ComponentBehavior: Bound
import QtQuick
import rille.ui

// Column headers of the track table: click sorts, dragging a header moves
// the column, dragging its right edge resizes it, right-click (or the
// button at the end) chooses the columns.
Rectangle {
    id: header
    property var columns: []
    property string sortKey: ""
    property bool descending: false
    signal sortRequested(string key)
    signal resized(int index, real width)
    signal resizeFinished()
    signal moved(int from, int to)
    signal chooserRequested()

    // While dragging a header: where it would land.
    property int dropIndex: -1

    height: 26
    color: Theme.panelRaised

    function indexAt(x) {
        var acc = 0
        for (var i = 0; i < columns.length; i++) {
            acc += columns[i].width
            if (x < acc - columns[i].width / 2)
                return i
        }
        return columns.length
    }
    function edgeX(i) {
        var acc = 0
        for (var k = 0; k < i && k < columns.length; k++)
            acc += columns[k].width
        return acc
    }

    Row {
        anchors.fill: parent
        // By count, not by the array: the cells (and a running drag) stay
        // alive while widths change.
        Repeater {
            model: header.columns.length
            Item {
                id: cell
                required property int index
                readonly property var modelData: header.columns[cell.index] || ({ title: "", sort: "", width: 0 })
                readonly property bool sorted: cell.modelData.sort.length > 0 && header.sortKey === cell.modelData.sort
                width: cell.modelData.width
                height: header.height

                UiText {
                    x: 6
                    // Room for the sort arrow so it never covers the title.
                    width: cell.width - 12 - (cell.sorted ? 14 : 0)
                    anchors.verticalCenter: parent.verticalCenter
                    text: cell.modelData.title
                    horizontalAlignment: cell.modelData.align === "right" ? Text.AlignRight : (cell.modelData.align === "center" ? Text.AlignHCenter : Text.AlignLeft)
                    color: cell.sorted ? Theme.text : Theme.textDim
                    font.pixelSize: Theme.fontSmall
                    font.bold: true
                    font.capitalization: Font.AllUppercase
                    elide: Text.ElideRight
                }
                Icon {
                    visible: cell.sorted
                    anchors.right: parent.right
                    anchors.rightMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    name: header.descending ? "chevron-down" : "chevron-up"
                    size: 11
                    color: Theme.sync
                }
                MouseArea {
                    id: area
                    anchors.fill: parent
                    anchors.rightMargin: 10
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    // The table scrolls in both directions; without this it
                    // would take the drag over as a scroll.
                    preventStealing: true
                    property real pressX: 0
                    property bool dragging: false
                    onPressed: mouse => {
                        pressX = mouse.x
                        dragging = false
                    }
                    onPositionChanged: mouse => {
                        if (!pressed)
                            return
                        if (Math.abs(mouse.x - pressX) > 8)
                            dragging = true
                        if (dragging)
                            header.dropIndex = header.indexAt(cell.x + mouse.x)
                    }
                    onReleased: mouse => {
                        if (dragging) {
                            var to = header.indexAt(cell.x + mouse.x)
                            header.dropIndex = -1
                            if (to !== cell.index && to !== cell.index + 1)
                                header.moved(cell.index, to > cell.index ? to - 1 : to)
                            dragging = false
                        }
                    }
                    onClicked: mouse => {
                        if (mouse.button === Qt.RightButton)
                            header.chooserRequested()
                        else if (!dragging && cell.modelData.sort.length)
                            header.sortRequested(cell.modelData.sort)
                    }
                }
                // Resize handle on the right edge.
                Rectangle {
                    anchors.right: parent.right
                    width: 1
                    height: parent.height - 10
                    anchors.verticalCenter: parent.verticalCenter
                    color: resize.containsMouse || resize.pressed ? Theme.sync : Theme.border
                }
                MouseArea {
                    id: resize
                    anchors.right: parent.right
                    width: 10
                    height: parent.height
                    hoverEnabled: true
                    preventStealing: true
                    cursorShape: Qt.SplitHCursor
                    property real startX: 0
                    property real startWidth: 0
                    onPressed: mouse => {
                        startX = mapToItem(header, mouse.x, 0).x
                        startWidth = cell.modelData.width
                    }
                    onPositionChanged: mouse => {
                        if (pressed)
                            header.resized(cell.index, Math.max(28, startWidth + mapToItem(header, mouse.x, 0).x - startX))
                    }
                    onReleased: header.resizeFinished()
                }
            }
        }
    }
    // Drop marker while moving a column.
    Rectangle {
        visible: header.dropIndex >= 0
        x: header.edgeX(header.dropIndex) - 1
        width: 3
        height: parent.height
        color: Theme.sync
    }
    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
}
