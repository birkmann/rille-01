pragma ComponentBehavior: Bound
import QtQuick
import rille.ui

// One row of the track table. The cells follow `columns` (the browser's
// column layout: order, width, kind of cell and model role).
Rectangle {
    id: row
    required property int index
    // All roles of the row (see TrackListModel).
    required property var model

    property var columns: []
    // Row → path token, for dragging files that are not in the collection.
    property var dragToken

    signal pressedRow(int row, int mods, int button)
    signal loadRow(int row, int deck)
    signal rate(var id, int stars)

    // Set by the browser's row size (compact, medium, large).
    property int rowHeight: 30
    property int coverSize: 26
    height: rowHeight
    // On a deck: tinted. Played in this session (and off the decks): faded.
    readonly property bool onDeck: row.model.deckMark.length > 0
    readonly property bool played: row.model.sessionPlayed && !row.onDeck
    readonly property color baseColor: row.index % 2 ? Theme.panel : Theme.rowAlt
    color: row.model.selected ? Theme.selection
        : (row.onDeck ? Qt.tint(row.baseColor, Qt.rgba(Theme.sync.r, Theme.sync.g, Theme.sync.b, 0.14)) : row.baseColor)

    Drag.active: dragArea.drag.active
    Drag.dragType: Drag.Automatic
    Drag.supportedActions: Qt.CopyAction
    Drag.mimeData: row.model.inCollection
        ? { "application/x-rille-track": String(row.model.trackId) }
        : (row.model.streamed
            ? { "application/x-rille-beatport": String(row.model.beatportId) }
            : { "application/x-rille-path": String(row.dragToken ? row.dragToken(row.index) : -1) })

    HoverHandler { id: rowHover }
    MouseArea {
        id: dragArea
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        // Touch (mobile layout): swiping scrolls the list, so no dragging,
        // a tap selects and press-and-hold opens the menu.
        drag.target: Theme.mobile ? null : dragProxy
        onPressed: mouse => {
            if (!Theme.mobile || mouse.button === Qt.RightButton)
                row.pressedRow(row.index, mouse.modifiers, mouse.button)
        }
        onClicked: mouse => {
            if (Theme.mobile && mouse.button === Qt.LeftButton)
                row.pressedRow(row.index, mouse.modifiers, mouse.button)
        }
        onPressAndHold: mouse => {
            if (Theme.mobile)
                row.pressedRow(row.index, 0, Qt.RightButton)
        }
        onDoubleClicked: row.loadRow(row.index, 0)
    }
    Item { id: dragProxy }

    Row {
        anchors.fill: parent
        // By count, not by the array, so resizing does not rebuild cells.
        Repeater {
            model: row.columns.length
            Loader {
                id: cell
                required property int index
                readonly property var modelData: row.columns[cell.index] || ({ kind: "text", width: 0 })
                width: cell.modelData.width
                height: row.rowHeight
                opacity: row.played && cell.modelData.kind !== "status" ? 0.45 : 1
                sourceComponent: {
                    switch (cell.modelData.kind) {
                    case "status": return statusCell
                    case "number": return numberCell
                    case "cover": return coverCell
                    case "title": return titleCell
                    case "bpm": return bpmCell
                    case "key": return keyCell
                    case "rating": return ratingCell
                    default: return textCell
                    }
                }
                Binding {
                    target: cell.item
                    property: "col"
                    value: cell.modelData
                    when: cell.status === Loader.Ready
                }
            }
        }
    }

    Component {
        id: textCell
        UiText {
            property var col: ({})
            leftPadding: 6
            rightPadding: 6
            text: col.role ? (row.model[col.role] === undefined ? "" : String(row.model[col.role])) : ""
            horizontalAlignment: col.align === "right" ? Text.AlignRight : (col.align === "center" ? Text.AlignHCenter : Text.AlignLeft)
            color: col.tone === "faint" ? Theme.textFaint : (col.tone === "dim" ? Theme.textDim : Theme.text)
            font.pixelSize: col.tone === "faint" ? Theme.fontSmall : Theme.fontNormal
            elide: col.key === "file" || col.key === "path" ? Text.ElideMiddle : Text.ElideRight
        }
    }
    Component {
        id: titleCell
        Item {
            property var col: ({})
            UiText {
                anchors.fill: parent
                leftPadding: 6
                rightPadding: hoverLoad.visible ? hoverLoad.width + 8 : 6
                text: row.model.title
                elide: Text.ElideRight
                color: row.model.missing ? Theme.danger : (row.model.inCollection || row.model.streamed ? Theme.text : Theme.textDim)
                font.bold: row.model.selected
            }
            // Like the decks on screen (A B on top, C D below) or in one row
            // (Settings → Decks → Load buttons).
            Grid {
                id: hoverLoad
                anchors.right: parent.right
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                columns: hoverLoad.twoRows ? 2 : 4
                rowSpacing: 2
                columnSpacing: 2
                // Touch: on the selected row.
                visible: rowHover.hovered || (Theme.mobile && row.model.selected)
                readonly property bool fourDecks: AppController.deckCount === 4
                readonly property bool twoRows: fourDecks && AppController.loadButtonsGrid
                readonly property int buttonWidth: Theme.mobile ? 32 : 24
                readonly property int buttonHeight: Math.min(Theme.mobile ? 28 : 20,
                    twoRows ? Math.floor((row.rowHeight - 4 - rowSpacing) / 2) : row.rowHeight)
                DjButton { text: "A"; tip: "Load to deck A"; implicitWidth: hoverLoad.buttonWidth; implicitHeight: hoverLoad.buttonHeight; onClicked: row.loadRow(row.index, 0) }
                DjButton { text: "B"; tip: "Load to deck B"; implicitWidth: hoverLoad.buttonWidth; implicitHeight: hoverLoad.buttonHeight; onClicked: row.loadRow(row.index, 1) }
                DjButton { visible: hoverLoad.fourDecks; text: "C"; tip: "Load to deck C"; implicitWidth: hoverLoad.buttonWidth; implicitHeight: hoverLoad.buttonHeight; onClicked: row.loadRow(row.index, 2) }
                DjButton { visible: hoverLoad.fourDecks; text: "D"; tip: "Load to deck D"; implicitWidth: hoverLoad.buttonWidth; implicitHeight: hoverLoad.buttonHeight; onClicked: row.loadRow(row.index, 3) }
            }
        }
    }
    Component {
        id: statusCell
        Item {
            property var col: ({})
            Row {
                anchors.centerIn: parent
                spacing: 3
                // Beatport: stream only, cached, kept offline, queued, downloading.
                Item {
                    id: offlineMark
                    readonly property string state_: row.model.offlineState
                    visible: state_.length > 0
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14
                    height: 14
                    Icon {
                        id: offlineIcon
                        anchors.centerIn: parent
                        size: 13
                        name: {
                            switch (offlineMark.state_) {
                            case "offline": return "check"
                            case "cached": return "check"
                            case "queued": return "clock"
                            default: return "cloud"
                            }
                        }
                        color: {
                            switch (offlineMark.state_) {
                            case "offline": return Theme.play
                            case "downloading": return Theme.sync
                            case "cached": return Theme.textDim
                            default: return Theme.textFaint
                            }
                        }
                        SequentialAnimation on opacity {
                            running: offlineMark.state_ === "downloading"
                            loops: Animation.Infinite
                            onStopped: offlineIcon.opacity = 1
                            NumberAnimation { from: 1; to: 0.4; duration: 600 }
                            NumberAnimation { from: 0.4; to: 1; duration: 600 }
                        }
                    }
                    // How far the download is.
                    Rectangle {
                        visible: offlineMark.state_ === "downloading"
                        anchors.bottom: parent.bottom
                        anchors.bottomMargin: -2
                        width: parent.width
                        height: 2
                        color: Theme.control
                        Rectangle {
                            width: parent.width * Math.max(0, row.model.downloadProgress)
                            height: parent.height
                            color: Theme.sync
                        }
                    }
                    HoverHandler { id: offlineHover }
                    Tip {
                        visible: offlineHover.hovered
                        text: {
                            switch (offlineMark.state_) {
                            case "offline": return "Downloaded: plays offline, kept until you remove it"
                            case "cached": return "In the cache: plays offline, may be removed to make room"
                            case "queued": return "Waiting to download"
                            case "downloading": return "Downloading " + Math.floor(row.model.downloadProgress * 100) + " %"
                            default: return "Streams from Beatport (downloads when loaded)"
                            }
                        }
                    }
                }
                Rectangle {
                    visible: row.onDeck
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(14, deckLabel.implicitWidth + 6)
                    height: 14
                    radius: 2
                    color: Theme.sync
                    UiText {
                        id: deckLabel
                        anchors.centerIn: parent
                        text: row.model.deckMark
                        color: Theme.textOnLit
                        font.bold: true
                        font.pixelSize: Theme.fontSmall
                    }
                    HoverHandler { id: deckHover }
                    Tip {
                        visible: deckHover.hovered
                        text: row.model.deckMark.length > 1
                            ? "Loaded on decks " + row.model.deckMark.split("").join(", ")
                            : "Loaded on deck " + row.model.deckMark
                    }
                }
                Icon {
                    visible: row.played
                    anchors.verticalCenter: parent.verticalCenter
                    name: "played"
                    size: 13
                    color: Theme.textDim
                    HoverHandler { id: playedHover }
                    Tip {
                        visible: playedHover.hovered
                        text: "Played in this session"
                    }
                }
                Icon {
                    // Catalog tracks are analyzed when they are loaded.
                    visible: row.model.deckMark.length === 0 && row.model.analysisState !== "done" && !(row.model.streamed && !row.model.inCollection)
                    anchors.verticalCenter: parent.verticalCenter
                    size: 12
                    name: {
                        switch (row.model.analysisState) {
                        case "running": return "analyze"
                        case "queued": return "clock"
                        case "failed": return "alert"
                        case "new": return "plus"
                        default: return "dot"
                        }
                    }
                    color: {
                        switch (row.model.analysisState) {
                        case "running": return Theme.sync
                        case "failed": return Theme.danger
                        case "stale":
                        case "none": return Theme.warn
                        default: return Theme.textFaint
                        }
                    }
                }
                Icon {
                    visible: row.model.locked
                    anchors.verticalCenter: parent.verticalCenter
                    name: "lock"
                    size: 11
                    color: Theme.textDim
                }
            }
        }
    }
    Component {
        id: numberCell
        UiText {
            property var col: ({})
            rightPadding: 6
            horizontalAlignment: Text.AlignRight
            text: row.model.rowNumber
            color: Theme.textFaint
            font.pixelSize: Theme.fontSmall
        }
    }
    Component {
        id: coverCell
        Item {
            property var col: ({})
            Rectangle { width: 3; height: row.rowHeight - 4; y: 2; radius: 1; color: row.model.tagColor.length ? row.model.tagColor : "transparent" }
            Rectangle {
                x: 6
                anchors.verticalCenter: parent.verticalCenter
                width: row.coverSize
                height: row.coverSize
                radius: 2
                color: Theme.control
                Image {
                    anchors.fill: parent
                    // The 256 px thumbnail once the small one would look soft.
                    source: row.coverSize > 32 && row.model.coverLarge.length > 0 ? row.model.coverLarge : row.model.cover
                    visible: row.model.cover.length > 0
                    asynchronous: true
                    sourceSize.width: row.coverSize * 2
                    sourceSize.height: row.coverSize * 2
                    fillMode: Image.PreserveAspectCrop
                }
                Icon {
                    anchors.centerIn: parent
                    visible: row.model.cover.length === 0
                    name: "music"
                    size: Math.round(row.coverSize * 0.45)
                    color: Theme.textFaint
                }
            }
        }
    }
    Component {
        id: bpmCell
        UiText {
            property var col: ({})
            rightPadding: 8
            horizontalAlignment: Text.AlignRight
            text: row.model.bpm
            color: row.model.gridAttention.length ? Theme.warn : Theme.text
            font.bold: row.model.gridAttention.length > 0
        }
    }
    Component {
        id: keyCell
        Item {
            property var col: ({})
            Rectangle {
                visible: row.model.keyText.length > 0
                anchors.centerIn: parent
                width: keyLabel.implicitWidth + 10
                height: 18
                radius: 3
                color: Qt.darker(row.model.keyColor, 2.6)
                border.color: row.model.keyColor
                UiText { id: keyLabel; anchors.centerIn: parent; text: row.model.keyText; color: row.model.keyColor; font.bold: true; font.pixelSize: Theme.fontSmall }
            }
        }
    }
    Component {
        id: ratingCell
        Row {
            property var col: ({})
            leftPadding: 5
            Repeater {
                model: 5
                Item {
                    id: star
                    required property int index
                    width: 12
                    height: row.rowHeight
                    Icon {
                        anchors.centerIn: parent
                        name: star.index < row.model.rating ? "star" : "dot"
                        size: star.index < row.model.rating ? 11 : 6
                        color: star.index < row.model.rating ? Theme.text : Theme.textFaint
                    }
                    MouseArea {
                        anchors.fill: parent
                        enabled: row.model.inCollection
                        onClicked: row.rate(row.model.trackId, star.index + 1 === row.model.rating ? 0 : star.index + 1)
                    }
                }
            }
        }
    }
}
