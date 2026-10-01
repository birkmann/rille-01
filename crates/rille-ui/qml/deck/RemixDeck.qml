pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import rille.ui

// A remix deck: four slots (columns) of sample cells, one page of four rows
// shown at a time. Click a cell to play it (in time with the deck), right-
// click to edit it, drop a track on it to load it. `dc` is its DeckController.
Rectangle {
    id: deck
    required property DeckController dc
    property bool compact: false
    readonly property string letter: String.fromCharCode(65 + dc.deck)
    readonly property var cells: JSON.parse(dc.remixCellsJson || "[]")
    readonly property var slots: JSON.parse(dc.remixSlotsJson || "[]")
    readonly property int page: dc.remixPage
    readonly property int loadedCount: cells.filter(c => c !== null).length
    // Queued cells blink with the beat, like the controller pads.
    readonly property bool beatOn: ((AppController.clockBeat % 1) + 1) % 1 < 0.5
    readonly property int transportHeight: compact ? 28 : 34

    function t(name) {
        return dc.target(name)
    }
    function tap(name) {
        AppController.press(t(name), true)
        AppController.press(t(name), false)
    }
    function slotState(i) {
        return slots[i] || { cell: -1, queued: -1, progress: 0, volume: 1, filter: 0.5, muted: false, meter: 0 }
    }
    // Bar and beat of the deck's position ("5.3"), counted from 1.
    function barText() {
        if (!dc.loaded)
            return "—"
        var b = Math.floor(dc.beat + 1e-6)
        return (Math.floor(b / 4) + 1) + "." + (((b % 4) + 4) % 4 + 1)
    }

    color: Theme.panel
    radius: Theme.radius
    clip: true
    border.color: Theme.panelEdge

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 6
        spacing: 4

        // --- Header: set, position, tempo ----------------------------------
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: deck.compact ? 40 : 50
            spacing: 8
            Rectangle {
                Layout.preferredWidth: deck.compact ? 40 : 50
                Layout.preferredHeight: deck.compact ? 40 : 50
                color: Theme.control
                radius: 3
                Grid {
                    anchors.centerIn: parent
                    columns: 4
                    spacing: 2
                    Repeater {
                        model: 16
                        Rectangle {
                            required property int index
                            readonly property var st: deck.slotState(index % 4)
                            width: deck.compact ? 6 : 8
                            height: width
                            radius: 1
                            color: st.cell >= 0 && Math.floor(index / 4) === (st.cell % 16) % 4 && Math.floor((st.cell % 16) / 4) === deck.page
                                ? Theme.sync : Theme.meterOff
                        }
                    }
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                UiText {
                    Layout.fillWidth: true
                    visible: !nameEdit.visible
                    text: deck.dc.title
                    font.pixelSize: Theme.fontLarge
                    font.bold: true
                    elide: Text.ElideRight
                    MouseArea {
                        id: titleArea
                        anchors.fill: parent
                        hoverEnabled: true
                        onDoubleClicked: {
                            nameEdit.text = deck.dc.title
                            nameEdit.visible = true
                            nameEdit.forceActiveFocus()
                            nameEdit.selectAll()
                        }
                    }
                    Tip { text: "Remix set name; double-click to rename"; visible: titleArea.containsMouse }
                }
                TextField {
                    id: nameEdit
                    visible: false
                    Layout.fillWidth: true
                    Layout.preferredHeight: 22
                    font.pixelSize: Theme.fontNormal
                    color: Theme.text
                    background: Rectangle { color: Theme.bg; border.color: Theme.sync; radius: 3 }
                    onAccepted: {
                        if (text.trim().length)
                            deck.dc.renameRemixSet(text)
                        visible = false
                    }
                    onActiveFocusChanged: if (!activeFocus) visible = false
                    Keys.onEscapePressed: visible = false
                }
                UiText {
                    Layout.fillWidth: true
                    text: "Remix deck · " + deck.loadedCount + (deck.loadedCount === 1 ? " sample" : " samples")
                        + " · page " + (deck.page + 1) + "/4"
                    color: Theme.textDim
                    elide: Text.ElideRight
                }
            }
            ColumnLayout {
                spacing: -2
                UiText {
                    Layout.alignment: Qt.AlignRight
                    text: deck.barText()
                    color: deck.dc.playing ? Theme.text : Theme.textDim
                    font.pixelSize: deck.compact ? 20 : Theme.fontHuge
                    font.family: Theme.fontMono
                    font.weight: Font.Medium
                    font.letterSpacing: -1
                }
                UiText {
                    Layout.alignment: Qt.AlignRight
                    text: "bar.beat"
                    color: Theme.textFaint
                    font.pixelSize: Theme.fontSmall
                }
            }
            ColumnLayout {
                spacing: -2
                Layout.preferredWidth: 86
                UiText {
                    Layout.alignment: Qt.AlignRight
                    text: deck.dc.bpm > 0 ? deck.dc.bpm.toFixed(2) : "—"
                    font.pixelSize: deck.compact ? 20 : Theme.fontHuge
                    font.family: Theme.fontMono
                    font.weight: Font.Medium
                    font.letterSpacing: -1
                }
                UiText {
                    Layout.alignment: Qt.AlignRight
                    text: (deck.dc.tempoPercent >= 0 ? "+" : "") + deck.dc.tempoPercent.toFixed(1) + "%"
                    color: Math.abs(deck.dc.tempoPercent) < 0.05 ? Theme.textDim : Theme.warn
                    font.pixelSize: Theme.fontSmall
                    font.bold: true
                }
            }
            UiText {
                text: deck.letter
                color: deck.dc.master ? Theme.accent : Theme.textDim
                font.pixelSize: deck.compact ? 24 : 28
                font.family: Theme.fontCondensed
                font.bold: true
            }
        }

        // --- Sync row ----------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            DjButton { text: "SYNC"; target: deck.t("sync"); lit: deck.dc.sync; litColor: Theme.sync; implicitHeight: 24; tip: "Sync: follow the tempo and beat phase of the master" }
            DjButton { text: "MASTER"; target: deck.t("master"); lit: deck.dc.master; litColor: Theme.accent; implicitHeight: 24; tip: "Master: this deck sets the tempo the others sync to" }
            // Phase to the master above the beat in the bar, as on a track deck.
            Item {
                Layout.fillWidth: true
                Layout.minimumWidth: 40
                Layout.preferredHeight: 24
                Rectangle {
                    id: phase
                    width: parent.width
                    height: 10
                    color: Theme.bg
                    radius: 2
                    border.color: Theme.border
                    Rectangle { anchors.centerIn: parent; width: 1; height: parent.height; color: Theme.textFaint }
                    Rectangle {
                        visible: deck.dc.playing && AppController.masterDeck >= 0 && AppController.masterDeck !== deck.dc.deck
                        height: parent.height - 2
                        y: 1
                        width: Math.max(2, Math.abs(deck.dc.phaseError) * parent.width)
                        x: deck.dc.phaseError < 0 ? parent.width / 2 - width : parent.width / 2
                        color: !deck.dc.sync ? Theme.textDim : (Math.abs(deck.dc.phaseError) < 0.02 ? Theme.sync : Theme.warn)
                        radius: 2
                    }
                }
                Row {
                    anchors.top: phase.bottom
                    anchors.topMargin: 3
                    width: parent.width
                    spacing: 3
                    Repeater {
                        model: 4
                        Rectangle {
                            required property int index
                            width: (parent.width - 9) / 4
                            height: 6
                            radius: 1
                            color: deck.dc.playing && deck.dc.beatInBar === index ? (index === 0 ? Theme.accent : Theme.text) : Theme.meterOff
                        }
                    }
                }
            }
            DjButton {
                text: "FROM " + deck.dc.remixSource
                implicitHeight: 24
                tip: "Capture source: right-click a cell (or CAPTURE + pad) to capture a loop from this deck: its active loop, or its loop size from the current beat. Click to pick another deck."
                onClicked: AppController.nudge(deck.t("remix_capture_source"), 1)
                onRightClicked: AppController.nudge(deck.t("remix_capture_source"), -1)
            }
            DjButton { icon: "lock"; text: "KEY"; target: deck.t("keylock"); lit: deck.dc.keylock; litColor: Theme.sync; implicitHeight: 24; tip: "Keylock: samples keep their pitch at any tempo" }
            DjButton { icon: "metronome"; target: deck.t("tick"); lit: deck.dc.tick; litColor: Theme.warn; implicitWidth: 28; implicitHeight: 24; tip: "Metronome on the deck's beats" }
            DjButton {
                icon: "sliders"
                implicitWidth: 28
                implicitHeight: 24
                tip: "Remix set"
                onClicked: setMenu.popup()
                StyledMenu {
                    id: setMenu
                    StyledMenuItem {
                        iconName: "list"
                        text: "Rename set…"
                        onTriggered: {
                            nameEdit.text = deck.dc.title
                            nameEdit.visible = true
                            nameEdit.forceActiveFocus()
                            nameEdit.selectAll()
                        }
                    }
                    StyledMenuSeparator {}
                    StyledMenuItem { iconName: "trash"; danger: true; text: "Clear all cells"; onTriggered: AppController.eject(deck.dc.deck) }
                }
            }
        }

        // --- Slots -------------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 4

            Repeater {
                model: 4
                // One slot: its playing sample, progress, cells, mixer.
                ColumnLayout {
                    id: slot
                    required property int index
                    readonly property var st: deck.slotState(index)
                    readonly property var playing: st.cell >= 0 ? deck.cells[st.cell] : null
                    readonly property int number: index + 1
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 1
                    spacing: 3

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 3
                        UiText {
                            Layout.fillWidth: true
                            text: slot.playing ? slot.playing.name : "Slot " + slot.number
                            color: slot.playing ? Theme.text : Theme.textFaint
                            font.bold: true
                            elide: Text.ElideRight
                        }
                        DjButton {
                            text: "M"
                            target: deck.t("remix_mute." + slot.number)
                            lit: slot.st.muted
                            litColor: Theme.danger
                            implicitWidth: 22
                            implicitHeight: 20
                            fontSize: Theme.fontTiny
                            tip: "Mute slot " + slot.number + " (it keeps playing silently)"
                        }
                        DjButton {
                            icon: "square"
                            target: deck.t("remix_stop." + slot.number)
                            lit: slot.st.cell >= 0
                            litColor: slot.playing ? slot.playing.color : Theme.accent
                            subtle: true
                            implicitWidth: 22
                            implicitHeight: 20
                            tip: "Stop slot " + slot.number
                        }
                    }

                    // Where the playing sample is.
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 5
                        radius: 2
                        color: Theme.bg
                        Rectangle {
                            visible: slot.playing !== null
                            width: parent.width * slot.st.progress
                            height: parent.height
                            radius: 2
                            color: slot.playing ? slot.playing.color : Theme.sync
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        spacing: 3

                        ColumnLayout {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            spacing: 2
                            Repeater {
                                model: 4
                                Rectangle {
                                    id: cellView
                                    required property int index
                                    readonly property int cell: slot.index * 16 + deck.page * 4 + index
                                    readonly property var sample: deck.cells[cell] || null
                                    readonly property bool isPlaying: slot.st.cell === cell
                                    readonly property bool isQueued: slot.st.queued === cell
                                    Layout.fillWidth: true
                                    Layout.fillHeight: true
                                    Layout.minimumHeight: 16
                                    radius: 3
                                    color: !sample ? (cellDrop.containsDrag ? Theme.panelRaised : Theme.bg)
                                        : (isPlaying ? sample.color : (cellArea.containsMouse ? Qt.darker(sample.color, 2.2) : Qt.darker(sample.color, 3.4)))
                                    border.color: cellDrop.containsDrag ? Theme.sync
                                        : (isQueued && deck.beatOn ? sample.color : (sample ? Qt.darker(sample.color, 1.8) : Theme.border))
                                    border.width: isQueued ? 2 : 1

                                    Row {
                                        anchors.verticalCenter: parent.verticalCenter
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        anchors.leftMargin: 5
                                        anchors.rightMargin: 4
                                        spacing: 4
                                        Icon {
                                            anchors.verticalCenter: parent.verticalCenter
                                            visible: cellView.sample !== null
                                            name: cellView.sample && cellView.sample.loop ? "loop" : "play"
                                            size: 12
                                            color: cellView.isPlaying ? Theme.textOnLit : (cellView.sample ? cellView.sample.color : Theme.textFaint)
                                        }
                                        UiText {
                                            anchors.verticalCenter: parent.verticalCenter
                                            width: parent.width - 16
                                            text: cellView.sample ? cellView.sample.name : (cellArea.containsMouse || cellDrop.containsDrag ? "drop a track" : "")
                                            color: cellView.isPlaying ? Theme.textOnLit : (cellView.sample ? (cellView.sample.ready ? Theme.text : Theme.textDim) : Theme.textFaint)
                                            font.pixelSize: Theme.fontSmall
                                            font.bold: cellView.isPlaying
                                            elide: Text.ElideRight
                                        }
                                    }

                                    MouseArea {
                                        id: cellArea
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                                        onPressed: mouse => {
                                            if (mouse.button === Qt.RightButton)
                                                cellMenu.open(cellView.cell)
                                            else
                                                AppController.press(deck.t("remix_cell." + (cellView.cell + 1)), true)
                                        }
                                        onReleased: mouse => {
                                            if (mouse.button === Qt.LeftButton)
                                                AppController.press(deck.t("remix_cell." + (cellView.cell + 1)), false)
                                        }
                                        onWheel: wheel => AppController.nudge(deck.t("remix_page"), wheel.angleDelta.y > 0 ? -1 : 1)
                                    }
                                    Tip {
                                        visible: cellArea.containsMouse && cellView.sample !== null
                                        text: cellView.sample
                                            ? cellView.sample.name + "\n" + (cellView.sample.loop ? "Loop" : "One-shot") + " · " + cellView.sample.bpm.toFixed(2) + " BPM"
                                              + "\nClick: play (on the next quantize step) · right-click: edit"
                                            : ""
                                    }
                                    DropArea {
                                        id: cellDrop
                                        anchors.fill: parent
                                        keys: ["application/x-rille-track", "application/x-rille-path", "text/uri-list"]
                                        onDropped: drop => {
                                            if (drop.hasUrls && drop.urls.length > 0)
                                                deck.dc.remixLoadUrl(cellView.cell, drop.urls[0])
                                            else if (drop.getDataAsString("application/x-rille-track").length)
                                                deck.dc.remixLoadTrack(cellView.cell, Number(drop.getDataAsString("application/x-rille-track")))
                                            else if (drop.getDataAsString("application/x-rille-path").length)
                                                deck.dc.remixLoadPath(cellView.cell, Number(drop.getDataAsString("application/x-rille-path")))
                                            drop.acceptProposedAction()
                                        }
                                    }
                                }
                            }
                        }

                        // Slot level, volume and filter.
                        ColumnLayout {
                            Layout.fillHeight: true
                            spacing: 2
                            RowLayout {
                                Layout.fillHeight: true
                                spacing: 2
                                VuMeter {
                                    Layout.fillHeight: true
                                    Layout.preferredWidth: 4
                                    level: slot.st.meter
                                    segments: 12
                                }
                                Fader {
                                    Layout.fillHeight: true
                                    Layout.preferredWidth: 16
                                    value: slot.st.volume
                                    defaultValue: 1.0
                                    target: deck.t("remix_volume." + slot.number)
                                    ticks: 5
                                    tip: "Slot " + slot.number + " volume"
                                }
                            }
                            Knob {
                                Layout.alignment: Qt.AlignHCenter
                                size: 22
                                value: slot.st.filter
                                defaultValue: 0.5
                                bipolar: true
                                target: deck.t("remix_filter." + slot.number)
                                format: v => Theme.filterText(v)
                                tip: "Slot " + slot.number + " filter; double-click resets"
                            }
                        }
                    }
                }
            }

            // The deck's own tempo (when not synced).
            ColumnLayout {
                Layout.fillHeight: true
                spacing: 2
                Fader {
                    Layout.fillHeight: true
                    Layout.preferredWidth: 22
                    value: deck.dc.tempoFader
                    target: deck.t("tempo")
                    centered: true
                    ticks: 9
                    color: Math.abs(deck.dc.tempoFader - 0.5) < 0.001 ? Theme.textDim : Theme.warn
                    tip: "Tempo ±" + Math.round(AppController.tempoRange * 100) + " %; double-click resets"
                }
                UiText {
                    Layout.alignment: Qt.AlignHCenter
                    text: "±" + Math.round(AppController.tempoRange * 100)
                    color: Theme.textFaint
                    font.pixelSize: Theme.fontTiny
                }
            }
        }

        // --- Transport ---------------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            DjButton {
                icon: deck.dc.playing ? "pause" : "play"
                target: deck.t("play")
                lit: true
                litColor: deck.dc.playing ? Theme.play : Qt.darker(Theme.play, 1.8)
                implicitWidth: 60
                implicitHeight: deck.transportHeight
                tip: "Play / pause the deck (a cell also starts it)"
            }
            DjButton { flat: true; text: "CUE"; target: deck.t("cue"); implicitHeight: deck.transportHeight; fontSize: Theme.fontLarge; tip: "Stop every slot and go back to bar 1" }
            Item { Layout.fillWidth: true }
            // Pages of the cell grid (the wheel over the cells changes them too).
            UiText { visible: deck.width > 520; text: "PAGE"; color: Theme.textFaint; font.pixelSize: Theme.fontSmall; font.bold: true }
            Repeater {
                model: 4
                DjButton {
                    id: pageButton
                    required property int index
                    flat: true
                    text: index + 1
                    lit: index === deck.page
                    litColor: Theme.sync
                    implicitWidth: 28
                    implicitHeight: deck.transportHeight
                    fontSize: Theme.fontNormal
                    tip: "Page " + (index + 1) + ": cell rows " + (index * 4 + 1) + "–" + (index * 4 + 4) + " (the controller's pads show one page)"
                    onClicked: AppController.nudge(deck.t("remix_page"), pageButton.index - deck.page)
                }
            }
            Item { Layout.preferredWidth: 6 }
            DjButton {
                flat: true
                text: "Q"
                target: deck.t("remix_quantize")
                lit: deck.dc.remixQuantize
                litColor: Theme.sync
                implicitHeight: deck.transportHeight
                fontSize: Theme.fontLarge
                tip: "Quantize: a played cell starts on the next step of this size"
            }
            // Quantize size, as the loop size selector of a track deck.
            Rectangle {
                implicitHeight: deck.transportHeight
                implicitWidth: quantRow.implicitWidth + 4
                color: Theme.control
                radius: 3
                RowLayout {
                    id: quantRow
                    anchors.fill: parent
                    anchors.margins: 2
                    spacing: 1
                    DjButton {
                        flat: true
                        icon: "chevron-left"
                        implicitWidth: 22
                        Layout.fillHeight: true
                        onClicked: AppController.nudge(deck.t("remix_quantize_size"), -1)
                    }
                    Repeater {
                        model: deck.dc.remixQuantCount()
                        DjButton {
                            id: quantButton
                            required property int index
                            visible: deck.width > 560 || Math.abs(index - deck.dc.remixQuantIndex) <= 1
                            flat: true
                            text: deck.dc.remixQuantLabel(index)
                            lit: index === deck.dc.remixQuantIndex
                            litColor: Theme.textDim
                            implicitWidth: 34
                            Layout.fillHeight: true
                            fontSize: Theme.fontNormal
                            tip: "Quantize to " + text + (Number(text) === 1 ? " beat" : (text.indexOf("/") >= 0 ? " beat" : " beats"))
                            onClicked: AppController.nudge(deck.t("remix_quantize_size"), quantButton.index - deck.dc.remixQuantIndex)
                        }
                    }
                    DjButton {
                        flat: true
                        icon: "chevron-right"
                        implicitWidth: 22
                        Layout.fillHeight: true
                        onClicked: AppController.nudge(deck.t("remix_quantize_size"), 1)
                    }
                }
            }
        }
    }

    // Edits one cell.
    StyledMenu {
        id: cellMenu
        property int cell: -1
        readonly property var sample: cell >= 0 ? deck.cells[cell] : null
        function open(c) {
            cell = c
            popup()
        }
        StyledMenuItem {
            caption: true
            text: "Slot " + (Math.floor(cellMenu.cell / 16) + 1) + ", cell " + (cellMenu.cell % 16 + 1) + (cellMenu.sample ? ": " + cellMenu.sample.name : "")
        }
        StyledMenuItem {
            iconName: "import"
            text: "Capture from deck " + deck.dc.remixSource
            onTriggered: deck.dc.remixCell(cellMenu.cell, "capture", 0)
        }
        StyledMenuItem {
            iconName: cellMenu.sample && cellMenu.sample.loop ? "play" : "loop"
            enabled: cellMenu.sample !== null
            text: cellMenu.sample && !cellMenu.sample.loop ? "Make it a loop" : "Make it a one-shot"
            onTriggered: deck.dc.remixCell(cellMenu.cell, "type", 0)
        }
        StyledMenu {
            title: "Color"
            enabled: cellMenu.sample !== null
            Repeater {
                model: deck.dc.remixColorCount()
                StyledMenuItem {
                    required property int index
                    swatch: deck.dc.remixColor(index)
                    text: index === 0 ? "White" : "Color " + index
                    onTriggered: deck.dc.remixCell(cellMenu.cell, "color", index)
                }
            }
        }
        StyledMenuSeparator {}
        StyledMenuItem {
            iconName: "trash"
            danger: true
            enabled: cellMenu.sample !== null
            text: "Remove sample"
            onTriggered: deck.dc.remixCell(cellMenu.cell, "delete", 0)
        }
    }
}
