pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import rille.ui

// Library browser: source tree (collection, playlists, file explorer, music
// folders, history) on the left, the track table on the right, a status
// bar with analysis progress below.
Item {
    id: browser

    // Current source row in the tree, and what the toolbar shows for it.
    property int sourceRow: 0
    property string sourceTitle: "Track Collection"
    readonly property bool folderMode: tracks.sourceKind === 3
    readonly property var trackColors: ["#e0565b", "#f5a524", "#f2c94c", "#5fd068", "#2ec4b6", "#4fb3e8", "#7b8cff", "#d96bd6"]
    readonly property var trackColorNames: ["Red", "Orange", "Yellow", "Green", "Teal", "Blue", "Violet", "Pink"]

    // --- Columns ----------------------------------------------------------------
    // Every column the table can show. kind: how the cell is drawn; role: the
    // model role of text cells; sort: TrackListModel sort key.
    readonly property var allColumns: [
        { key: "status", title: "", name: "Status", kind: "status", width: 40, sort: "" },
        { key: "number", title: "#", name: "Number", kind: "number", width: 40, sort: "", align: "right" },
        { key: "cover", title: "", name: "Cover art", kind: "cover", width: 38, sort: "" },
        { key: "title", title: "Title", flex: true, kind: "title", width: 240, sort: "title" },
        { key: "artist", title: "Artist", flex: true, kind: "text", role: "artist", width: 200, sort: "artist" },
        { key: "remixer", title: "Remixer", flex: true, kind: "text", role: "remixer", width: 150, sort: "remixer" },
        { key: "album", title: "Album", flex: true, kind: "text", role: "album", width: 170, sort: "album" },
        { key: "label", title: "Label", flex: true, kind: "text", role: "label", width: 150, sort: "label", tone: "dim" },
        { key: "genre", title: "Genre", flex: true, kind: "text", role: "genre", width: 120, sort: "genre", tone: "dim" },
        { key: "bpm", title: "BPM", kind: "bpm", width: 66, sort: "bpm", align: "right" },
        { key: "key", title: "Key", kind: "key", width: 52, sort: "key", align: "center" },
        { key: "rating", title: "Rating", kind: "rating", width: 74, sort: "rating" },
        { key: "duration", title: "Time", kind: "text", role: "duration", width: 56, sort: "duration", tone: "dim", align: "right" },
        { key: "year", title: "Year", kind: "text", role: "year", width: 56, sort: "year", tone: "dim", align: "right" },
        { key: "comment", title: "Comment", flex: true, kind: "text", role: "comment", width: 200, sort: "comment", tone: "dim" },
        { key: "bitrate", title: "Bitrate", kind: "text", role: "bitrate", width: 80, sort: "bitrate", tone: "dim", align: "right" },
        { key: "samplerate", title: "Sample rate", kind: "text", role: "sampleRate", width: 90, sort: "samplerate", tone: "dim", align: "right" },
        { key: "size", title: "Size", kind: "text", role: "fileSize", width: 76, sort: "size", tone: "dim", align: "right" },
        { key: "added", title: "Added", kind: "text", role: "dateAdded", width: 92, sort: "added", tone: "dim" },
        { key: "played", title: "Last played", kind: "text", role: "lastPlayed", width: 120, sort: "played", tone: "dim" },
        { key: "plays", title: "Plays", kind: "text", role: "playCount", width: 56, sort: "plays", tone: "dim", align: "right" },
        { key: "match", title: "Match", name: "Match (suggestions)", kind: "text", role: "match", width: 60, sort: "", tone: "dim", align: "right" },
        { key: "file", title: "File", flex: true, kind: "text", role: "fileName", width: 240, sort: "file", tone: "faint" },
        { key: "path", title: "Path", flex: true, kind: "text", role: "filePath", width: 340, sort: "path", tone: "faint" }
    ]
    readonly property var defaultLayout: [
        { key: "status", width: 40 }, { key: "number", width: 40 }, { key: "cover", width: 38 },
        { key: "title", width: 143 }, { key: "artist", width: 126 }, { key: "album", width: 140 },
        { key: "genre", width: 167 }, { key: "rating", width: 74 }, { key: "key", width: 52 },
        { key: "duration", width: 56 }, { key: "bpm", width: 66 }
    ]
    // Shown columns in order: [{ key, width }], saved in the settings.
    property var layout: []
    // Row size: 0 compact, 1 medium, 2 large (rows and cover art grow).
    property int rowSize: 0
    readonly property var rowHeights: [30, 44, 68]
    readonly property var coverSizes: [26, 40, 64]
    readonly property int rowHeight: rowHeights[rowSize] || 30
    readonly property int coverSize: coverSizes[rowSize] || 26
    property real viewWidth: 800
    // The shown columns with their definitions. Text columns (flex) share any
    // spare width in proportion to their set widths, so the table looks
    // the same at every window size; the others keep their width.
    readonly property var columns: {
        var out = []
        var fixed = 0
        var flex = 0
        for (var i = 0; i < layout.length; i++) {
            var def = columnDef(layout[i].key)
            if (!def)
                continue
            var c = Object.assign({}, def)
            c.width = setWidth(layout[i])
            if (c.flex)
                flex += c.width
            else
                fixed += c.width
            out.push(c)
        }
        var scale = flex > 0 ? Math.max(1, (viewWidth - fixed) / flex) : 1
        for (var k = 0; k < out.length; k++) {
            if (out[k].flex)
                out[k].width = Math.floor(out[k].width * scale)
        }
        return out
    }
    readonly property real tableWidth: {
        var w = 0
        for (var i = 0; i < columns.length; i++)
            w += columns[i].width
        return Math.max(w, viewWidth)
    }

    // A column's set width; the cover column fits the cover size.
    function setWidth(l) {
        return l.key === "cover" ? coverSize + 12 : l.width
    }
    function setRowSize(size) {
        rowSize = size
        AppController.setSetting("browser_row_size", String(size))
    }
    function columnDef(key) {
        for (var i = 0; i < allColumns.length; i++)
            if (allColumns[i].key === key)
                return allColumns[i]
        return null
    }
    function isShown(key) {
        return layout.some(l => l.key === key)
    }
    function defaultColumns() {
        return defaultLayout.map(l => ({ key: l.key, width: l.width }))
    }
    function loadLayout() {
        var saved = []
        try {
            saved = JSON.parse(JSON.parse(AppController.settingsJson()).browser_columns || "[]")
        } catch (e) {
            saved = []
        }
        saved = saved.filter(l => l && columnDef(l.key))
        layout = saved.length ? saved : defaultColumns()
        rowSize = Number(JSON.parse(AppController.settingsJson()).browser_row_size) || 0
    }
    function saveLayout() {
        AppController.setSetting("browser_columns", JSON.stringify(layout))
    }
    function toggleColumn(key) {
        if (isShown(key)) {
            if (layout.length > 1)
                layout = layout.filter(l => l.key !== key)
        } else {
            layout = layout.concat([{ key: key, width: columnDef(key).width }])
        }
        saveLayout()
    }
    function moveColumn(from, to) {
        var l = layout.slice()
        var item = l.splice(from, 1)[0]
        l.splice(Math.max(0, Math.min(l.length, to)), 0, item)
        layout = l
        saveLayout()
    }
    // `shown` is the width the user dragged to. For a stretched text column,
    // store the set width that is shown that wide after sharing out the
    // spare space again (the other columns keep their set widths).
    function resizeColumn(i, shown) {
        var fixed = 0
        var otherFlex = 0
        for (var j = 0; j < layout.length; j++) {
            var def = columnDef(layout[j].key)
            if (!def)
                continue
            if (!def.flex)
                fixed += j === i ? 0 : setWidth(layout[j])
            else if (j !== i)
                otherFlex += layout[j].width
        }
        var set = shown
        var mine = columnDef(layout[i].key)
        if (mine && mine.flex && otherFlex > 0) {
            var avail = viewWidth - fixed
            if (avail - shown > 20) {
                var candidate = shown * otherFlex / (avail - shown)
                // Only while the table is stretched (no sideways scrolling).
                if (avail / (otherFlex + candidate) >= 1)
                    set = candidate
            }
        }
        var l = layout.slice()
        l[i] = { key: l[i].key, width: Math.max(28, Math.round(set)) }
        layout = l
    }
    function resetColumns() {
        layout = defaultColumns()
        saveLayout()
    }
    // Opens a folder (path token) in the explorer view.
    function showFolder(token) {
        if (token < 0)
            return
        tracks.sourceKind = 3
        tracks.sourceId = token
        sourceRow = -1
        sourceTitle = AppController.tokenPathText(token)
        tracks.refresh()
        list.currentIndex = -1
    }

    function loadSelected(deck) {
        if (list.currentIndex >= 0)
            tracks.loadRow(list.currentIndex, deck)
    }

    // The tree row of the open source again, after the tree changed (rows
    // added or removed above it); -1 when it is not shown.
    function syncSourceRow() {
        if (sourceRow < 0 && folderMode)
            return
        for (var r = 0; r < tree.count; r++) {
            if (tree.kindAt(r) === tracks.sourceKind && tree.idAt(r) === tracks.sourceId) {
                sourceRow = r
                return
            }
        }
        sourceRow = -1
    }

    function openSource(row) {
        var kind = tree.kindAt(row)
        if (kind === 4 || kind === 5) {
            tree.toggle(row)
            return
        }
        if (kind < 0)
            return
        sourceRow = row
        sourceTitle = kind === 3 ? tree.pathTextAt(row) : tree.labelAt(row)
        tracks.sourceKind = kind
        tracks.sourceId = tree.idAt(row)
        tracks.refresh()
        list.currentIndex = -1
    }

    // `rille --browse=<folder>`: start in that folder.
    Component.onCompleted: {
        loadLayout()
        // `--row-menu` / `--column-menu`: open a menu (for screenshots).
        if (AppController.hasArg("row-menu") || AppController.hasArg("column-menu"))
            menuDemo.start()
        var token = AppController.browseToken()
        if (token >= 0) {
            tracks.sourceKind = 3
            tracks.sourceId = token
            sourceRow = -1
            sourceTitle = AppController.tokenPathText(token)
            tracks.refresh()
        }
    }

    Timer {
        id: menuDemo
        interval: 1500
        onTriggered: {
            var m = AppController.hasArg("row-menu") ? rowMenu : columnMenu
            if (m === rowMenu) {
                tracks.select(0, 0)
                rowMenu.open(0)
            } else {
                columnMenu.popup()
            }
            browser.Window.window.grabItem = m.background.parent
        }
    }

    TrackListModel {
        id: tracks
        sortKey: "artist"
        Component.onCompleted: refresh()
    }
    BrowserTreeModel {
        id: tree
        Component.onCompleted: refresh()
        onModelReset: browser.syncSourceRow()
    }
    Connections {
        target: AppController
        function onLibraryRevisionChanged() {
            tracks.refresh()
            tree.refresh()
            // Suggestions switched off while showing them.
            if (tracks.sourceKind === 6 && !AppController.suggestionsEnabled)
                browser.openSource(0)
        }
        function onSuggestionsRevisionChanged() {
            if (tracks.sourceKind === 6)
                tracks.refresh()
        }
        function onTracksRevisionChanged() {
            tracks.updateChanged()
        }
        // Browser controls from a MIDI controller.
        function onBrowserActionSeqChanged() {
            var a = AppController.browserAction
            if (a.startsWith("load:")) {
                browser.loadSelected(Number(a.substring(5)))
            } else if (a.startsWith("cell:")) {
                // "cell:<deck>:<cell>": the selected track into a remix cell.
                var parts = a.split(":")
                if (list.currentIndex >= 0)
                    tracks.loadRowToCell(list.currentIndex, Number(parts[1]), Number(parts[2]))
            } else if (a.startsWith("scroll:")) {
                list.currentIndex = Math.max(0, Math.min(list.count - 1, list.currentIndex + Math.round(Number(a.substring(7)))))
                tracks.select(list.currentIndex, 0)
            } else if (a.startsWith("tree:")) {
                treeView.currentIndex = Math.max(0, Math.min(treeView.count - 1, treeView.currentIndex + Math.round(Number(a.substring(5)))))
            } else if (a === "toggle") {
                browser.openSource(treeView.currentIndex)
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Theme.gap

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.gap

            // --- Source tree ------------------------------------------------
            Panel {
                Layout.preferredWidth: 250
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 4
                    spacing: 4

                    SearchField {
                        id: search
                        Layout.fillWidth: true
                        onTextEdited: searchDelay.restart()
                        Keys.onDownPressed: list.forceActiveFocus()
                        Timer {
                            id: searchDelay
                            interval: 150
                            onTriggered: {
                                tracks.search = search.text
                                tracks.refresh()
                            }
                        }
                    }

                    ListView {
                        id: treeView
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: tree
                        currentIndex: 0
                        boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: StyledScrollBar { id: treeBar }
                        delegate: Rectangle {
                            id: node
                            required property int index
                            required property string label
                            required property int kind
                            required property var nodeId
                            required property int depth
                            required property bool expandable
                            required property bool expanded
                            required property string icon
                            required property string detail
                            readonly property bool isSource: node.index === browser.sourceRow && node.kind !== 4
                            // Leave room for the scrollbar so it doesn't cover the counts.
                            width: ListView.view.width - (treeBar.needed ? treeBar.thickness + 2 : 0)
                            height: node.kind === 4 ? 26 : 24
                            radius: 2
                            color: node.isSource ? Theme.selection : (nodeHover.hovered ? Theme.panelRaised : "transparent")

                            Item {
                                id: arrow
                                x: 2 + node.depth * 14
                                width: 16
                                height: parent.height
                                Icon {
                                    anchors.centerIn: parent
                                    visible: node.expandable
                                    name: node.expanded ? "chevron-down" : "chevron-right"
                                    size: 12
                                    color: Theme.textDim
                                }
                                MouseArea {
                                    anchors.fill: parent
                                    enabled: node.expandable
                                    onClicked: tree.toggle(node.index)
                                }
                            }
                            Icon {
                                id: nodeIcon
                                anchors.verticalCenter: parent.verticalCenter
                                x: arrow.x + arrow.width + 2
                                name: node.icon
                                size: 14
                                color: node.isSource ? Theme.sync : Theme.textDim
                            }
                            UiText {
                                anchors.verticalCenter: parent.verticalCenter
                                x: nodeIcon.x + nodeIcon.width + 6
                                width: node.width - x - detailText.width - 10
                                text: node.label
                                elide: Text.ElideRight
                                color: node.kind === 4 ? Theme.textDim : Theme.text
                                font.pixelSize: node.kind === 4 ? Theme.fontSmall : Theme.fontNormal
                                font.bold: node.kind === 4 || node.kind === 0
                                font.capitalization: node.kind === 4 ? Font.AllUppercase : Font.MixedCase
                                font.letterSpacing: node.kind === 4 ? 0.6 : 0
                            }
                            UiText {
                                id: detailText
                                anchors.verticalCenter: parent.verticalCenter
                                anchors.right: parent.right
                                anchors.rightMargin: 6
                                text: node.detail
                                color: Theme.textFaint
                                font.pixelSize: Theme.fontSmall
                            }
                            HoverHandler { id: nodeHover }
                            MouseArea {
                                anchors.fill: parent
                                anchors.leftMargin: arrow.x + arrow.width
                                acceptedButtons: Qt.LeftButton | Qt.RightButton
                                onClicked: mouse => {
                                    treeView.currentIndex = node.index
                                    if (mouse.button === Qt.RightButton) {
                                        treeMenu.kind = node.kind
                                        treeMenu.nodeId = node.nodeId
                                        treeMenu.popup()
                                    } else {
                                        browser.openSource(node.index)
                                    }
                                }
                                onDoubleClicked: {
                                    if (node.expandable)
                                        tree.toggle(node.index)
                                }
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 4
                        TextField {
                            id: newName
                            Layout.fillWidth: true
                            implicitHeight: 24
                            placeholderText: "New playlist"
                            placeholderTextColor: Theme.textFaint
                            color: Theme.text
                            font.pixelSize: Theme.fontSmall
                            background: Rectangle { color: Theme.bg; radius: Theme.radius; border.color: newName.activeFocus ? Theme.sync : Theme.border }
                            onAccepted: {
                                if (text.length) {
                                    AppController.createPlaylist(text)
                                    text = ""
                                }
                            }
                        }
                        DjButton {
                            icon: "plus"
                            tip: "Create the playlist"
                            implicitWidth: 26
                            implicitHeight: 24
                            onClicked: newName.accepted()
                        }
                    }
                }
            }

            // --- Tracks -------------------------------------------------------
            Panel {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 4
                    spacing: 2

                    // Toolbar: where we are and what can be done here.
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 28
                        spacing: 4
                        Icon {
                            name: browser.folderMode ? "folder" : (tracks.sourceKind === 1 ? "list" : (tracks.sourceKind === 2 ? "clock" : (tracks.sourceKind === 6 ? "suggest" : "library")))
                            size: 16
                            color: Theme.sync
                        }
                        UiText {
                            Layout.fillWidth: true
                            text: browser.sourceTitle
                            elide: Text.ElideMiddle
                            font.pixelSize: Theme.fontNormal
                            font.bold: true
                        }
                        UiText {
                            text: tracks.summary
                            color: Theme.textDim
                            font.pixelSize: Theme.fontSmall
                        }
                        // Row size: compact, medium, large.
                        Row {
                            spacing: 0
                            Repeater {
                                model: [
                                    { text: "S", tip: "Compact rows" },
                                    { text: "M", tip: "Taller rows with larger cover art" },
                                    { text: "L", tip: "Large rows with big cover art" }
                                ]
                                DjButton {
                                    required property int index
                                    required property var modelData
                                    flat: true
                                    text: modelData.text
                                    tip: modelData.tip
                                    implicitWidth: 24
                                    lit: browser.rowSize === index
                                    onClicked: browser.setRowSize(index)
                                }
                            }
                        }
                        DjButton {
                            icon: "columns"
                            text: "COLUMNS"
                            onClicked: columnMenu.popup()
                        }
                        DjButton {
                            visible: browser.folderMode
                            icon: "import"
                            text: tracks.selectedCount > 0 ? "IMPORT SELECTED" : "IMPORT"
                            enabled: tracks.newCount > 0
                            tip: "Add the tracks to the collection without analyzing them"
                            onClicked: tracks.selectedCount > 0 ? tracks.importSelected(false) : AppController.importFolder(tracks.sourceId, false, false)
                        }
                        DjButton {
                            visible: browser.folderMode
                            icon: "analyze"
                            text: "IMPORT + ANALYZE"
                            tip: "Add the tracks to the collection and analyze beatgrid, key and loudness"
                            onClicked: tracks.selectedCount > 0 ? tracks.analyzeSelected(false) : AppController.analyzeFolder(tracks.sourceId, false, false)
                        }
                        DjButton {
                            visible: browser.folderMode && browser.sourceRow >= 0 && !tree.isMusicFolder(browser.sourceRow)
                            icon: "music"
                            text: "ADD AS MUSIC FOLDER"
                            tip: "Add this folder to the music folders: its tracks join the collection and new ones are picked up on every scan"
                            onClicked: AppController.addMusicFolderToken(tracks.sourceId)
                        }
                        DjButton {
                            visible: browser.folderMode
                            icon: "external"
                            tip: "Open the folder in the file manager"
                            implicitWidth: 28
                            onClicked: AppController.openFolder(tracks.sourceId)
                        }
                        DjButton {
                            visible: !browser.folderMode && tracks.selectedCount > 0
                            icon: "analyze"
                            text: "ANALYZE " + tracks.selectedCount
                            onClicked: tracks.analyzeSelected(false)
                        }
                    }

                    ListView {
                        id: list
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: tracks
                        focus: true
                        currentIndex: -1
                        highlightMoveDuration: 0
                        boundsBehavior: Flickable.StopAtBounds
                        reuseItems: true
                        // Wider than the view when the columns need it.
                        contentWidth: browser.tableWidth
                        flickableDirection: Flickable.HorizontalAndVerticalFlick
                        // Scrollbars beside the rows, not over them: the vertical one
                        // starts below the header, the rows end before it, and the
                        // last row can scroll above the horizontal one.
                        ScrollBar.vertical: StyledScrollBar { id: vbar; topPadding: 28 }
                        ScrollBar.horizontal: StyledScrollBar { id: hbar; rightPadding: vbar.thickness + 2 }
                        bottomMargin: hbar.needed ? hbar.thickness : 0
                        // A fixed gutter for the vertical bar (making it depend on
                        // whether the bar is needed would loop through the widths).
                        readonly property real rowsWidth: width - vbar.thickness
                        onRowsWidthChanged: browser.viewWidth = rowsWidth
                        Component.onCompleted: browser.viewWidth = rowsWidth
                        headerPositioning: ListView.OverlayHeader
                        header: TrackHeader {
                            z: 2
                            width: browser.tableWidth
                            columns: browser.columns
                            sortKey: tracks.sortKey
                            descending: tracks.descending
                            onSortRequested: key => tracks.sortBy(key)
                            onResized: (i, w) => browser.resizeColumn(i, w)
                            onResizeFinished: browser.saveLayout()
                            onMoved: (from, to) => browser.moveColumn(from, to)
                            onChooserRequested: columnMenu.popup()
                        }
                        Keys.onPressed: event => {
                            if (event.key === Qt.Key_Left && (event.modifiers & Qt.ShiftModifier)) {
                                browser.loadSelected(0)
                                event.accepted = true
                            } else if (event.key === Qt.Key_Right && (event.modifiers & Qt.ShiftModifier)) {
                                browser.loadSelected(1)
                                event.accepted = true
                            } else if (event.key === Qt.Key_A && (event.modifiers & Qt.ControlModifier)) {
                                tracks.selectAll()
                                event.accepted = true
                            } else if (event.key === Qt.Key_Up || event.key === Qt.Key_Down) {
                                var next = currentIndex + (event.key === Qt.Key_Up ? -1 : 1)
                                if (next >= 0 && next < count) {
                                    currentIndex = next
                                    tracks.select(next, (event.modifiers & Qt.ShiftModifier) ? 2 : 0)
                                }
                                event.accepted = true
                            } else if (event.key === Qt.Key_Menu && currentIndex >= 0) {
                                rowMenu.open(currentIndex)
                                event.accepted = true
                            }
                        }
                        delegate: TrackRow {
                            width: browser.tableWidth
                            columns: browser.columns
                            rowHeight: browser.rowHeight
                            coverSize: browser.coverSize
                            onPressedRow: (row, mods, button) => {
                                list.currentIndex = row
                                list.forceActiveFocus()
                                if (button === Qt.RightButton) {
                                    if (!tracks.isSelected(row))
                                        tracks.select(row, 0)
                                    rowMenu.open(row)
                                } else {
                                    tracks.select(row, (mods & Qt.ShiftModifier) ? 2 : ((mods & Qt.ControlModifier) ? 1 : 0))
                                }
                            }
                            onLoadRow: (row, deck) => tracks.loadRow(row, deck)
                            onRate: (id, stars) => AppController.setRating(id, stars)
                            dragToken: row => tracks.pathToken(row)
                        }
                    }
                }
            }
        }

        // --- Status bar -------------------------------------------------------
        Panel {
            Layout.fillWidth: true
            Layout.preferredHeight: 26
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 4
                spacing: 8

                // Analysis progress.
                RowLayout {
                    visible: AppController.analysisTotal > 0
                    spacing: 6
                    Icon { name: "analyze"; size: 14; color: AppController.analysisPaused ? Theme.textDim : Theme.sync }
                    UiText {
                        text: (AppController.analysisPaused ? "Paused " : "Analyzing ") + (AppController.analysisDone + AppController.analysisFailed) + " / " + AppController.analysisTotal
                        font.pixelSize: Theme.fontSmall
                        font.bold: true
                    }
                    Rectangle {
                        implicitWidth: 140
                        implicitHeight: 6
                        radius: 3
                        color: Theme.control
                        Rectangle {
                            width: parent.width * Math.min(1, (AppController.analysisDone + AppController.analysisFailed) / Math.max(1, AppController.analysisTotal))
                            height: parent.height
                            radius: 3
                            color: AppController.analysisPaused ? Theme.textDim : Theme.sync
                        }
                    }
                    UiText {
                        text: AppController.analysisTime
                        font.pixelSize: Theme.fontSmall
                    }
                    UiText {
                        Layout.maximumWidth: 320
                        text: AppController.analysisCurrent
                        color: Theme.textDim
                        font.pixelSize: Theme.fontSmall
                        elide: Text.ElideRight
                    }
                    DjButton {
                        icon: AppController.analysisPaused ? "play" : "pause"
                        implicitWidth: 24
                        implicitHeight: 20
                        tip: AppController.analysisPaused ? "Resume the analysis" : "Pause the analysis"
                        onClicked: AppController.pauseAnalysis(!AppController.analysisPaused)
                    }
                    DjButton {
                        icon: "x"
                        implicitWidth: 24
                        implicitHeight: 20
                        tip: "Cancel the analysis"
                        onClicked: AppController.cancelAnalysis()
                    }
                }
                UiText {
                    visible: AppController.analysisFailed > 0
                    text: AppController.analysisFailed + " failed"
                    color: Theme.danger
                    font.pixelSize: Theme.fontSmall
                }
                UiText {
                    visible: AppController.importText.length > 0
                    text: AppController.importText
                    color: Theme.sync
                    font.pixelSize: Theme.fontSmall
                }
                UiText {
                    Layout.fillWidth: true
                    text: AppController.status
                    color: Theme.textDim
                    font.pixelSize: Theme.fontSmall
                    elide: Text.ElideLeft
                    horizontalAlignment: Text.AlignRight
                }
            }
        }
    }

    // --- Menus ---------------------------------------------------------------
    // --- Track menu ------------------------------------------------------------
    StyledMenu {
        id: rowMenu
        property int row: -1
        readonly property bool inCollection: tracks.trackId(row) >= 0
        function open(r) {
            row = r
            popup()
        }
        StyledMenuItem { caption: true; text: tracks.selectionTitle(rowMenu.row) }
        StyledMenuItem { iconName: "play"; text: "Load on deck A"; onTriggered: tracks.loadRow(rowMenu.row, 0) }
        StyledMenuItem { iconName: "play"; text: "Load on deck B"; onTriggered: tracks.loadRow(rowMenu.row, 1) }
        StyledMenuItem { iconName: "play"; text: "Load on deck C"; visible: AppController.deckCount === 4; height: visible ? implicitHeight : 0; onTriggered: tracks.loadRow(rowMenu.row, 2) }
        StyledMenuItem { iconName: "play"; text: "Load on deck D"; visible: AppController.deckCount === 4; height: visible ? implicitHeight : 0; onTriggered: tracks.loadRow(rowMenu.row, 3) }
        StyledMenuSeparator {}
        StyledMenuItem { iconName: "analyze"; text: tracks.selectedCount > 1 ? "Analyze " + tracks.selectedCount + " tracks" : "Analyze"; onTriggered: tracks.analyzeSelected(false) }
        StyledMenuItem { iconName: "refresh"; text: "Analyze again"; onTriggered: tracks.analyzeSelected(true) }
        StyledMenuItem { iconName: "grid"; text: "Reset beatgrid"; enabled: rowMenu.inCollection; onTriggered: tracks.resetGridSelected() }
        StyledMenuSeparator {}
        StyledMenu {
            title: "Rating"
            enabled: rowMenu.inCollection
            Repeater {
                model: 6
                StyledMenuItem {
                    required property int index
                    iconName: index > 0 ? "star" : "x"
                    text: index > 0 ? "★★★★★".substring(0, index) : "No rating"
                    onTriggered: tracks.setRatingSelected(index)
                }
            }
        }
        StyledMenu {
            title: "Color"
            enabled: rowMenu.inCollection
            Repeater {
                model: browser.trackColorNames.length
                StyledMenuItem {
                    required property int index
                    swatch: browser.trackColors[index]
                    text: browser.trackColorNames[index]
                    onTriggered: tracks.setColorSelected(parseInt(browser.trackColors[index].substring(1), 16))
                }
            }
            StyledMenuSeparator {}
            StyledMenuItem { iconName: "x"; text: "No color"; onTriggered: tracks.setColorSelected(-1) }
        }
        StyledMenu {
            id: playlistMenu
            title: "Add to playlist"
            enabled: rowMenu.inCollection
            StyledMenuItem {
                iconName: "plus"
                text: "New playlist from selection"
                onTriggered: {
                    var id = AppController.createPlaylist("New playlist")
                    if (id >= 0)
                        tracks.addSelectedToPlaylist(id)
                }
            }
            StyledMenuSeparator {}
            Instantiator {
                model: tree
                delegate: StyledMenuItem {
                    required property string label
                    required property int kind
                    required property var nodeId
                    iconName: "list"
                    text: label
                    visible: kind === 1
                    height: visible ? implicitHeight : 0
                    onTriggered: tracks.addSelectedToPlaylist(nodeId)
                }
                onObjectAdded: (index, object) => playlistMenu.insertItem(index + 2, object)
                onObjectRemoved: (index, object) => playlistMenu.removeItem(object)
            }
        }
        StyledMenuSeparator {}
        StyledMenuItem {
            iconName: "folder"
            text: "Show in Explorer"
            onTriggered: browser.showFolder(tracks.folderToken(rowMenu.row))
        }
        StyledMenuItem {
            iconName: "external"
            text: "Open folder in file manager"
            onTriggered: AppController.openFolder(tracks.folderToken(rowMenu.row))
        }
        StyledMenuItem {
            iconName: "import"
            text: "Import to collection"
            visible: !rowMenu.inCollection
            height: visible ? implicitHeight : 0
            onTriggered: tracks.importSelected(false)
        }
        StyledMenuSeparator {}
        StyledMenuItem { iconName: "trash"; danger: true; text: "Remove from collection"; enabled: rowMenu.inCollection; onTriggered: tracks.removeSelected() }
    }

    // --- Column chooser ---------------------------------------------------------
    StyledMenu {
        id: columnMenu
        StyledMenuItem { caption: true; text: "Columns" }
        Repeater {
            model: browser.allColumns
            StyledMenuItem {
                required property var modelData
                marked: browser.isShown(modelData.key)
                text: modelData.name || modelData.title
                onTriggered: browser.toggleColumn(modelData.key)
            }
        }
        StyledMenuSeparator {}
        StyledMenuItem { iconName: "refresh"; text: "Reset to default"; onTriggered: browser.resetColumns() }
    }

    StyledMenu {
        id: treeMenu
        property int kind: -1
        property var nodeId: 0
        StyledMenuItem {
            text: "Import folder"
            visible: treeMenu.kind === 3
            height: visible ? implicitHeight : 0
            onTriggered: AppController.importFolder(treeMenu.nodeId, false, false)
        }
        StyledMenuItem {
            text: "Import with subfolders and analyze"
            visible: treeMenu.kind === 3
            height: visible ? implicitHeight : 0
            onTriggered: AppController.importFolder(treeMenu.nodeId, true, true)
        }
        StyledMenuItem {
            text: "Import as playlist"
            visible: treeMenu.kind === 3
            height: visible ? implicitHeight : 0
            onTriggered: AppController.importFolderAsPlaylist(treeMenu.nodeId, false, true)
        }
        StyledMenuItem {
            text: "Import with subfolders as playlists"
            visible: treeMenu.kind === 3
            height: visible ? implicitHeight : 0
            onTriggered: AppController.importFolderAsPlaylist(treeMenu.nodeId, true, true)
        }
        StyledMenuItem {
            text: "Analyze folder"
            visible: treeMenu.kind === 3
            height: visible ? implicitHeight : 0
            onTriggered: AppController.analyzeFolder(treeMenu.nodeId, false, false)
        }
        StyledMenuItem {
            text: "Add as music folder"
            visible: treeMenu.kind === 3
            height: visible ? implicitHeight : 0
            onTriggered: AppController.addMusicFolderToken(treeMenu.nodeId)
        }
        StyledMenuItem {
            text: "Open in file manager"
            visible: treeMenu.kind === 3
            height: visible ? implicitHeight : 0
            onTriggered: AppController.openFolder(treeMenu.nodeId)
        }
        StyledMenuItem {
            text: "Delete playlist"
            visible: treeMenu.kind === 1
            height: visible ? implicitHeight : 0
            onTriggered: AppController.deletePlaylist(treeMenu.nodeId)
        }
        StyledMenuItem {
            text: "Rescan music folders"
            visible: treeMenu.kind === 0
            height: visible ? implicitHeight : 0
            onTriggered: AppController.rescan()
        }
        StyledMenuItem {
            text: "Turn off suggestions"
            visible: treeMenu.kind === 6
            height: visible ? implicitHeight : 0
            onTriggered: AppController.setSetting("suggestions", "false")
        }
    }
}
