// Mouse tests for the track table header (resize, reorder, sort).
// Run with scripts/qmltest.sh.
import QtQuick
import QtTest
import rille.ui

Item {
    id: root
    width: 800
    height: 300
    property var cols: [
        { key: "a", title: "A", sort: "a", width: 100 },
        { key: "b", title: "B", sort: "b", width: 150 },
        { key: "c", title: "C", sort: "c", width: 120 }
    ]
    property int moves: 0
    property string sorted: ""
    ListView {
        id: list
        anchors.fill: parent
        model: 50
        contentWidth: 1200
        flickableDirection: Flickable.HorizontalAndVerticalFlick
        headerPositioning: ListView.OverlayHeader
        header: TrackHeader {
            id: hdr
            width: 1200
            columns: root.cols
            onResized: (i, w) => {
                var c = root.cols.slice()
                c[i] = Object.assign({}, c[i], { width: w })
                root.cols = c
            }
            onMoved: (from, to) => {
                var c = root.cols.slice()
                var it = c.splice(from, 1)[0]
                c.splice(to, 0, it)
                root.cols = c
                root.moves++
            }
            onSortRequested: key => root.sorted = key
        }
        delegate: Rectangle { width: 1200; height: 30; color: "#222" }
    }

    TestCase {
        name: "TrackHeader"
        when: windowShown
        function init() {
            root.cols = [
                { key: "a", title: "A", sort: "a", width: 100 },
                { key: "b", title: "B", sort: "b", width: 150 },
                { key: "c", title: "C", sort: "c", width: 120 }
            ]
            root.moves = 0
            wait(50)
        }
        function test_resize() {
            var h = list.headerItem
            // Grab 4 px left of column A's right edge and drag 60 px right in steps.
            var x = 96, y = 13
            mousePress(h, x, y)
            for (var k = 1; k <= 12; k++)
                mouseMove(h, x + k * 5, y)
            mouseRelease(h, x + 60, y)
            compare(Math.round(root.cols[0].width), 160, "column A follows the drag")
            compare(list.contentX, 0, "the list did not scroll")
        }
        function test_move() {
            var h = list.headerItem
            // Drag column B's header (x 100..250) to the start.
            mousePress(h, 170, 13)
            for (var k = 1; k <= 20; k++)
                mouseMove(h, 170 - k * 8, 13)
            mouseRelease(h, 20, 13)
            compare(root.moves, 1)
            compare(root.cols[0].key, "b")
        }
        function test_sort() {
            var h = list.headerItem
            mouseClick(h, 20, 13)
            compare(root.sorted, root.cols[0].sort)
        }
    }
}
