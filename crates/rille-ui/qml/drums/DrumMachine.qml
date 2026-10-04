pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Dialogs
import QtQuick.Layouts
import rille.ui

// The drum machine: eight instruments, sixteen steps, always in time with
// the master clock. One sequencer row (the selected instrument) or four;
// the instrument tabs choose what the row and the knobs edit. Every control
// sends a `drum.*` target, so MIDI learn works on all of them.
Rectangle {
    id: drums
    readonly property var st: JSON.parse(AppController.drumsJson || "{}")
    readonly property var inst: st.inst || []
    readonly property int sel: st.selected || 0
    readonly property bool four: AppController.drumsRows === 4
    // Four rows: the half of the instruments the selected one is in.
    readonly property int pageStart: sel < 4 ? 0 : 4
    readonly property var hits: AppController.drumsHits.split(",")
    readonly property bool beatOn: ((AppController.clockBeat % 1) + 1) % 1 < 0.5
    readonly property bool external: AppController.mixerChannels.length > 0
    // Phone width: the steps get a row of their own (two lines of eight),
    // the sound, channel and kit go below them.
    readonly property bool narrow: width < 700
    // Mobile landscape: the steps beside the transport, sound, channel and
    // kit in a second row.
    readonly property bool twoRows: Theme.mobile && !narrow
    // Wide screens: the layout stops growing here and sits in the middle,
    // so the steps keep their shape instead of stretching across the panel.
    readonly property int maxContentWidth: 1400
    readonly property var fullNames: ["Bass drum", "Snare drum", "Closed hi-hat", "Open hi-hat", "Clap", "Rim shot", "Tom", "Cymbal"]

    function instName(i) {
        return drums.inst[i] ? drums.inst[i].name : ""
    }
    function instColor(i) {
        return drums.inst[i] ? drums.inst[i].color : Theme.sync
    }
    function tap(target) {
        AppController.press(target, true)
        AppController.press(target, false)
    }
    function two(n) {
        return n < 10 ? "0" + n : "" + n
    }

    color: Theme.panel
    radius: Theme.radius
    border.color: Theme.panelEdge
    clip: true

    // A sequencer row: steps 1–16 in four beats. Click toggles, drag paints
    // (on or off, as the first step became), right-click or Shift toggles
    // the accent. `single`: the row of the selected instrument, using the
    // `drum.step.N` targets a controller's step buttons learn.
    component StepRow: Item {
        id: row
        required property int instIndex
        property bool single: true
        readonly property string steps: (drums.st.rows || [])[instIndex] || "................"
        readonly property int length: drums.st.length || 16
        readonly property int groupGap: 6
        // Narrow: steps 1–8 above 9–16.
        readonly property int lines: drums.narrow ? 2 : 1
        readonly property int perLine: 16 / lines
        readonly property real stepWidth: (width - (perLine - 1) * 3 - (perLine / 4 - 1) * groupGap) / perLine
        readonly property real lineHeight: (height - (lines - 1) * groupGap) / lines
        property int hoverStep: -1

        function stepX(s) {
            var c = s % perLine
            return c * (stepWidth + 3) + Math.floor(c / 4) * groupGap
        }
        function stepY(s) {
            return Math.floor(s / perLine) * (lineHeight + groupGap)
        }
        function stepAt(x, y) {
            var first = y > lineHeight + groupGap / 2 ? perLine * Math.min(lines - 1, Math.floor((y + groupGap / 2) / (lineHeight + groupGap))) : 0
            for (var s = first; s < first + perLine - 1; s++) {
                if (x < stepX(s) + stepWidth + 1.5)
                    return s
            }
            return first + perLine - 1
        }
        function target(kind, s) {
            if (single)
                return "drum." + kind + "." + (s + 1)
            return "drum." + (kind === "step" ? "cell" : "cell_accent") + "." + (instIndex * 16 + s + 1)
        }

        Repeater {
            model: 16
            DrumStep {
                required property int index
                x: row.stepX(index)
                y: row.stepY(index)
                width: row.stepWidth
                height: row.lineHeight
                number: index + 1
                on: row.steps[index] !== "."
                accent: row.steps[index] === "X"
                playhead: AppController.drumsStep === index
                active: index < row.length
                hovered: row.hoverStep === index
                tint: drums.instColor(row.instIndex)
                showNumber: !drums.four
            }
        }
        MouseArea {
            id: paint
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            property int last: -1
            property bool value: true
            onPressed: mouse => {
                var s = row.stepAt(mouse.x, mouse.y)
                if (mouse.button === Qt.RightButton || (mouse.modifiers & Qt.ShiftModifier)) {
                    drums.tap(row.target("accent", s))
                    last = -1
                    return
                }
                value = row.steps[s] === "."
                last = s
                AppController.setValue(row.target("step", s), value ? 1 : 0)
            }
            onPositionChanged: mouse => {
                row.hoverStep = row.stepAt(mouse.x, mouse.y)
                if (pressed && last >= 0 && row.hoverStep !== last) {
                    last = row.hoverStep
                    AppController.setValue(row.target("step", last), value ? 1 : 0)
                }
            }
            onReleased: last = -1
            onExited: row.hoverStep = -1
        }
    }

    // One row; narrow: transport, steps, sound and channel, kit below
    // each other (cells apart from the wide layout's, so they never clash).
    GridLayout {
        id: grid
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.topMargin: 6
        anchors.bottomMargin: 6
        width: Math.min(drums.width - 12, drums.maxContentWidth)
        columnSpacing: 10
        rowSpacing: drums.narrow || drums.twoRows ? 6 : 0

        // --- Transport and pattern -----------------------------------------
        ColumnLayout {
            Layout.row: 0
            Layout.column: 0
            Layout.alignment: Qt.AlignVCenter
            Layout.fillWidth: false
            Layout.fillHeight: false
            spacing: 4
            RowLayout {
                spacing: 4
                DjButton {
                    icon: AppController.drumsPlaying ? "pause" : "play"
                    target: "drum.play"
                    lit: AppController.drumsPlaying
                    litColor: Theme.play
                    implicitWidth: 52
                    implicitHeight: 30
                    tip: "Start or stop the drums. They start in time with the master: the pattern lands where it would be had it been running"
                }
                DjButton {
                    icon: "dot"
                    text: "REC"
                    target: "drum.record"
                    lit: AppController.drumsRecord
                    litColor: Theme.danger
                    implicitHeight: 30
                    tip: "Live recording: while the drums play, every instrument you play (▸, or pads on a controller) is written to the nearest step"
                }
            }
            RowLayout {
                spacing: 2
                DjButton {
                    flat: true
                    icon: "chevron-left"
                    implicitWidth: 22
                    implicitHeight: 26
                    onClicked: AppController.nudge("drum.pattern_select", -1)
                }
                Rectangle {
                    implicitWidth: 64
                    implicitHeight: 26
                    radius: 3
                    color: patternArea.containsMouse ? Theme.panelRaised : Theme.bg
                    border.color: drums.st.queued >= 0 ? (drums.beatOn ? Theme.sync : Theme.border) : Theme.border
                    UiText {
                        anchors.centerIn: parent
                        text: drums.st.queued >= 0
                            ? "P" + drums.two(drums.st.current + 1) + "›" + drums.two(drums.st.queued + 1)
                            : "PAT " + drums.two((drums.st.current || 0) + 1)
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.fontNormal
                        font.weight: Font.Medium
                        color: drums.st.queued >= 0 && drums.beatOn ? Theme.sync : Theme.text
                    }
                    MouseArea {
                        id: patternArea
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: patternPopup.open()
                        onWheel: wheel => AppController.nudge("drum.pattern_select", wheel.angleDelta.y > 0 ? -1 : 1)
                    }
                    Tip {
                        visible: patternArea.containsMouse
                        text: "Pattern: click to choose one of 16, scroll to step through. While playing, the next pattern starts when this one comes round"
                    }
                }
                DjButton {
                    flat: true
                    icon: "chevron-right"
                    implicitWidth: 22
                    implicitHeight: 26
                    onClicked: AppController.nudge("drum.pattern_select", 1)
                }
            }
        }

        // Length and swing.
        ColumnLayout {
            Layout.row: 0
            Layout.column: 1
            Layout.alignment: Qt.AlignVCenter
            Layout.fillWidth: false
            Layout.fillHeight: false
            spacing: 2
            UiText {
                Layout.alignment: Qt.AlignHCenter
                text: "LENGTH"
                color: Theme.textFaint
                font.pixelSize: Theme.fontTiny
                font.bold: true
            }
            RowLayout {
                spacing: 0
                DjButton {
                    flat: true
                    icon: "minus"
                    implicitWidth: 20
                    implicitHeight: 22
                    onClicked: AppController.nudge("drum.length", -1)
                }
                UiText {
                    Layout.preferredWidth: 22
                    horizontalAlignment: Text.AlignHCenter
                    text: drums.st.length || 16
                    font.family: Theme.fontMono
                    font.weight: Font.Medium
                    MouseArea {
                        anchors.fill: parent
                        onWheel: wheel => AppController.nudge("drum.length", wheel.angleDelta.y > 0 ? 1 : -1)
                    }
                }
                DjButton {
                    flat: true
                    icon: "plus"
                    implicitWidth: 20
                    implicitHeight: 22
                    onClicked: AppController.nudge("drum.length", 1)
                }
            }
        }
        Knob {
            Layout.row: 0
            Layout.column: 2
            Layout.alignment: drums.narrow ? Qt.AlignVCenter | Qt.AlignLeft : Qt.AlignVCenter
            label: "SWING"
            size: 30
            value: drums.st.swing || 0
            defaultValue: 0
            target: "drum.swing"
            format: v => Math.round(50 + v * 25) + " %"
            tip: "Swing: the off-beat sixteenths come later (50 % straight … 75 %)"
        }

        // --- Instruments and steps -----------------------------------------
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.row: drums.narrow ? 1 : 0
            Layout.column: drums.narrow ? 0 : 3
            Layout.columnSpan: drums.narrow ? 3 : 1
            Layout.minimumWidth: drums.narrow ? 0 : 280
            // Narrow: pads up to about 64 px tall, the rest of the room below.
            Layout.maximumHeight: drums.narrow ? (Theme.mobile ? 34 : 26) + (drums.four ? 4 : 1) * (2 * 64 + 6 + 4) : Number.POSITIVE_INFINITY
            spacing: 4
            RowLayout {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.mobile ? 30 : 22
                Layout.fillHeight: false
                spacing: 3
                Repeater {
                    model: 8
                    DrumInstTab {
                        required property int index
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        inst: index
                        name: drums.instName(index)
                        fullName: drums.fullNames[index]
                        tint: drums.instColor(index)
                        selected: drums.sel === index
                        used: ((drums.st.rows || [])[index] || ".").replace(/\./g, "").length > 0
                        muted: drums.inst[index] ? drums.inst[index].muted : false
                        loaded: drums.inst[index] ? drums.inst[index].loaded : true
                        hits: drums.hits[index] || ""
                        onMenuRequested: instMenu.openFor(index)
                    }
                }
            }
            Repeater {
                model: drums.four ? 4 : 1
                RowLayout {
                    id: seqRow
                    required property int index
                    readonly property int instIndex: drums.four ? drums.pageStart + index : drums.sel
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: 6
                    // Four rows: name and mute of each.
                    Rectangle {
                        visible: drums.four
                        Layout.preferredWidth: 58
                        Layout.fillHeight: true
                        radius: 3
                        color: drums.sel === seqRow.instIndex ? Qt.darker(drums.instColor(seqRow.instIndex), 2.8) : "transparent"
                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 6
                            anchors.rightMargin: 2
                            spacing: 2
                            UiText {
                                Layout.fillWidth: true
                                text: drums.instName(seqRow.instIndex)
                                color: drums.instColor(seqRow.instIndex)
                                font.pixelSize: Theme.fontSmall
                                font.bold: true
                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: drums.tap("drum.inst." + (seqRow.instIndex + 1))
                                }
                            }
                            DjButton {
                                text: "M"
                                implicitWidth: 22
                                implicitHeight: 20
                                fontSize: Theme.fontTiny
                                target: "drum.inst_mute." + (seqRow.instIndex + 1)
                                lit: drums.inst[seqRow.instIndex] ? drums.inst[seqRow.instIndex].muted : false
                                litColor: Theme.warn
                                tip: "Mute " + drums.fullNames[seqRow.instIndex]
                            }
                        }
                    }
                    StepRow {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        instIndex: seqRow.instIndex
                        single: !drums.four
                    }
                }
            }
        }

        // --- The selected instrument's sound -------------------------------
        ColumnLayout {
            Layout.row: drums.narrow ? 2 : (drums.twoRows ? 1 : 0)
            Layout.column: drums.narrow || drums.twoRows ? 0 : 4
            Layout.alignment: Qt.AlignVCenter
            Layout.fillWidth: false
            Layout.fillHeight: false
            spacing: 0
            RowLayout {
                spacing: 4
                UiText {
                    text: drums.fullNames[drums.sel] || ""
                    color: drums.instColor(drums.sel)
                    font.pixelSize: Theme.fontTiny
                    font.bold: true
                }
                Item { Layout.fillWidth: true }
                DjButton {
                    text: "M"
                    implicitWidth: 22
                    implicitHeight: 18
                    fontSize: Theme.fontTiny
                    target: "drum.inst_mute." + (drums.sel + 1)
                    lit: drums.inst[drums.sel] ? drums.inst[drums.sel].muted : false
                    litColor: Theme.warn
                    tip: "Mute the selected instrument"
                }
            }
            RowLayout {
                spacing: 0
                Knob {
                    label: "TUNE"
                    size: 30
                    bipolar: true
                    color: drums.instColor(drums.sel)
                    value: drums.inst[drums.sel] ? drums.inst[drums.sel].tune : 0.5
                    target: "drum.sel_tune"
                    format: v => {
                        var st = Math.round((v - 0.5) * 24)
                        return (st > 0 ? "+" : "") + st + " st"
                    }
                    tip: "Pitch of the selected instrument, ±12 semitones"
                }
                Knob {
                    label: "DECAY"
                    size: 30
                    defaultValue: 1
                    color: drums.instColor(drums.sel)
                    value: drums.inst[drums.sel] ? drums.inst[drums.sel].decay : 1
                    target: "drum.sel_decay"
                    format: v => v >= 0.999 ? "FULL" : Math.round(v * 100) + "%"
                    tip: "Length of the selected instrument's sound: right = the whole sample, left = short"
                }
                Knob {
                    label: "LEVEL"
                    size: 30
                    defaultValue: 1
                    color: drums.instColor(drums.sel)
                    value: drums.inst[drums.sel] ? drums.inst[drums.sel].level : 1
                    target: "drum.sel_level"
                    tip: "Level of the selected instrument"
                }
            }
        }

        // --- The drums' channel ---------------------------------------------
        RowLayout {
            Layout.row: drums.narrow ? 2 : (drums.twoRows ? 1 : 0)
            Layout.column: drums.narrow || drums.twoRows ? 1 : 5
            Layout.columnSpan: drums.narrow || drums.twoRows ? 2 : 1
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredHeight: 70
            Layout.fillWidth: false
            Layout.fillHeight: false
            spacing: 2
            Knob {
                label: "FILTER"
                size: 30
                bipolar: true
                color: Theme.warn
                value: drums.st.filter !== undefined ? drums.st.filter : 0.5
                target: "drum.filter"
                format: Theme.filterText
                tip: "Filter on the drums: left = low-pass, right = high-pass"
            }
            Knob {
                label: "LEVEL"
                size: 30
                defaultValue: 0.8
                value: drums.st.level !== undefined ? drums.st.level : 0.8
                target: "drum.level"
                tip: "Drum level in the main mix (not on the crossfader)"
            }
            ColumnLayout {
                Layout.fillHeight: true
                spacing: 2
                DjButton {
                    text: "FX1"
                    implicitWidth: 38
                    implicitHeight: 20
                    fontSize: Theme.fontTiny
                    target: "drum.fx_assign.1"
                    lit: drums.st.fx ? drums.st.fx[0] : false
                    litColor: Theme.fx
                    tip: "Send the drums through FX unit 1"
                }
                DjButton {
                    text: "FX2"
                    implicitWidth: 38
                    implicitHeight: 20
                    fontSize: Theme.fontTiny
                    target: "drum.fx_assign.2"
                    lit: drums.st.fx ? drums.st.fx[1] : false
                    litColor: Theme.fx
                    tip: "Send the drums through FX unit 2"
                }
                DjButton {
                    icon: "headphones"
                    implicitWidth: 38
                    implicitHeight: 20
                    target: "drum.pfl"
                    lit: !!drums.st.pfl
                    litColor: Theme.sync
                    tip: "Drums in the headphones"
                }
            }
            VuMeter {
                Layout.fillHeight: true
                Layout.preferredWidth: 5
                level: AppController.drumsMeter
                segments: 14
            }
        }

        Item {
            visible: drums.narrow
            Layout.row: 4
            Layout.column: 0
            Layout.fillHeight: true
        }

        // --- Kit ---------------------------------------------------------------
        ColumnLayout {
            Layout.row: drums.narrow ? 3 : (drums.twoRows ? 1 : 0)
            Layout.column: drums.narrow ? 0 : (drums.twoRows ? 3 : 6)
            Layout.columnSpan: drums.narrow ? 3 : 1
            Layout.alignment: drums.twoRows ? Qt.AlignVCenter | Qt.AlignRight : Qt.AlignVCenter
            Layout.fillWidth: drums.narrow
            Layout.fillHeight: false
            Layout.preferredWidth: drums.narrow ? -1 : 150
            spacing: 4
            StyledCombo {
                id: kitCombo
                Layout.fillWidth: true
                readonly property var kits: drums.st.kits || []
                model: kits.map(k => k.name)
                currentIndex: kits.findIndex(k => k.name === drums.st.kit)
                onActivated: idx => AppController.selectDrumKit(kits[idx].name)
            }
            RowLayout {
                spacing: 3
                DjButton {
                    icon: "list"
                    implicitWidth: 30
                    implicitHeight: 24
                    tip: "Kit, sounds and patterns"
                    onClicked: kitMenu.popup()
                }
                Item { Layout.fillWidth: true }
                DjButton {
                    text: "1"
                    implicitWidth: 26
                    implicitHeight: 24
                    lit: !drums.four
                    litColor: Theme.textDim
                    tip: "One sequencer row: the selected instrument"
                    onClicked: AppController.setSetting("drums_rows", "1")
                }
                DjButton {
                    text: "4"
                    implicitWidth: 26
                    implicitHeight: 24
                    lit: drums.four
                    litColor: Theme.textDim
                    tip: "Four sequencer rows"
                    onClicked: AppController.setSetting("drums_rows", "4")
                }
            }
            UiText {
                Layout.fillWidth: true
                visible: drums.external
                text: "Not on the hardware mixer"
                color: Theme.warn
                font.pixelSize: Theme.fontTiny
                elide: Text.ElideRight
            }
        }
    }

    // Choose a pattern.
    Popup {
        id: patternPopup
        x: grid.x
        y: drums.narrow ? 40 : drums.height - 6
        padding: 6
        background: Rectangle { color: Theme.panelRaised; border.color: Theme.border; radius: 6 }
        Grid {
            columns: 8
            spacing: 3
            Repeater {
                model: 16
                DjButton {
                    id: patButton
                    required property int index
                    readonly property bool queued: drums.st.queued === index
                    implicitWidth: 34
                    implicitHeight: 28
                    text: index + 1
                    target: "drum.pattern." + (index + 1)
                    lit: drums.st.current === index || (queued && drums.beatOn)
                    litColor: queued ? Theme.sync : Theme.textDim
                    onClicked: patternPopup.close()
                    // A dot for patterns with steps.
                    Rectangle {
                        visible: (drums.st.used || [])[patButton.index] === true
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.margins: 3
                        width: 4
                        height: 4
                        radius: 2
                        color: Theme.sync
                    }
                }
            }
        }
    }

    // Sound and steps of one instrument (right-click on its tab).
    StyledMenu {
        id: instMenu
        property int instIndex: 0
        function openFor(i) {
            instIndex = i
            popup()
        }
        StyledMenuItem {
            caption: true
            text: drums.fullNames[instMenu.instIndex] + " · " + (drums.st.kit || "")
        }
        StyledMenuItem {
            iconName: "play"
            text: "Play"
            onTriggered: drums.tap("drum.trigger." + (instMenu.instIndex + 1))
        }
        StyledMenuItem {
            iconName: "import"
            text: "Load a sound…"
            onTriggered: {
                sampleDialog.instIndex = instMenu.instIndex
                sampleDialog.open()
            }
        }
        StyledMenuItem {
            iconName: "music"
            text: "Load the selected track"
            onTriggered: AppController.drumLoadSelected(instMenu.instIndex)
        }
        StyledMenuItem {
            iconName: "trash"
            enabled: !drums.st.factory && !!drums.inst[instMenu.instIndex] && drums.inst[instMenu.instIndex].loaded
            text: "Remove the sound"
            onTriggered: AppController.drumClearSample(instMenu.instIndex)
        }
        StyledMenuSeparator {}
        StyledMenuItem {
            marked: drums.inst[instMenu.instIndex] ? drums.inst[instMenu.instIndex].muted : false
            text: "Mute"
            onTriggered: drums.tap("drum.inst_mute." + (instMenu.instIndex + 1))
        }
        StyledMenuItem {
            iconName: "x"
            danger: true
            text: "Clear its steps"
            onTriggered: {
                drums.tap("drum.inst." + (instMenu.instIndex + 1))
                drums.tap("drum.clear")
            }
        }
    }

    // Kit and pattern actions.
    StyledMenu {
        id: kitMenu
        StyledMenuItem {
            caption: true
            text: "Kit: " + (drums.st.kit || "") + (drums.st.factory ? " (factory)" : "")
        }
        StyledMenuItem {
            iconName: "plus"
            text: "New kit from this one…"
            onTriggered: newKit.open()
        }
        StyledMenuItem {
            iconName: "import"
            text: "Load a sound into " + drums.instName(drums.sel) + "…"
            onTriggered: {
                sampleDialog.instIndex = drums.sel
                sampleDialog.open()
            }
        }
        StyledMenuItem {
            iconName: "music"
            text: "Load the selected track into " + drums.instName(drums.sel)
            onTriggered: AppController.drumLoadSelected(drums.sel)
        }
        StyledMenuItem {
            iconName: "folder"
            text: "Open the kits folder"
            onTriggered: AppController.openDrumKitsFolder()
        }
        StyledMenuSeparator {}
        StyledMenuItem {
            text: "Copy pattern"
            onTriggered: AppController.drumPattern("copy")
        }
        StyledMenuItem {
            enabled: !!drums.st.clipboard
            text: "Paste pattern"
            onTriggered: AppController.drumPattern("paste")
        }
        StyledMenuItem {
            iconName: "x"
            danger: true
            text: "Clear " + drums.instName(drums.sel) + " steps"
            onTriggered: drums.tap("drum.clear")
        }
        StyledMenuItem {
            iconName: "x"
            danger: true
            text: "Clear pattern"
            onTriggered: drums.tap("drum.clear_pattern")
        }
        StyledMenuSeparator { visible: !Theme.mobile; height: visible ? implicitHeight : 0 }
        StyledMenuItem {
            visible: !Theme.mobile
            height: visible ? implicitHeight : 0
            iconName: AppController.drumsPosition === 0 ? "chevron-down" : "chevron-up"
            text: AppController.drumsPosition === 0 ? "Move below the decks" : "Move above the decks"
            onTriggered: AppController.setSetting("drums_position", AppController.drumsPosition === 0 ? "1" : "0")
        }
        StyledMenuItem {
            visible: !Theme.mobile
            height: visible ? implicitHeight : 0
            iconName: "eye-off"
            text: "Hide the drum machine"
            onTriggered: AppController.setSetting("drums_visible", "false")
        }
    }

    // Name a new kit.
    Popup {
        id: newKit
        anchors.centerIn: Overlay.overlay
        modal: true
        padding: 14
        property string error: ""
        onOpened: {
            error = ""
            kitName.text = "My " + (drums.st.kit || "kit")
            kitName.forceActiveFocus()
            kitName.selectAll()
        }
        background: Rectangle { color: Theme.panelRaised; border.color: Theme.border; radius: 8 }
        function create() {
            var e = AppController.newDrumKit(kitName.text)
            if (e.length)
                error = e
            else
                close()
        }
        ColumnLayout {
            spacing: 8
            UiText {
                text: "New kit from " + (drums.st.kit || "")
                font.pixelSize: Theme.fontLarge
                font.bold: true
            }
            UiText {
                text: "Its sounds are copied to a folder of its own, where you can replace them."
                color: Theme.textDim
            }
            TextField {
                id: kitName
                Layout.preferredWidth: 320
                color: Theme.text
                font.pixelSize: Theme.fontNormal
                background: Rectangle { color: Theme.bg; border.color: kitName.activeFocus ? Theme.sync : Theme.border; radius: 3 }
                onAccepted: newKit.create()
                Keys.onEscapePressed: newKit.close()
            }
            UiText {
                visible: newKit.error.length > 0
                text: newKit.error
                color: Theme.danger
            }
            RowLayout {
                Layout.alignment: Qt.AlignRight
                DjButton {
                    text: "Cancel"
                    onClicked: newKit.close()
                }
                DjButton {
                    text: "Create"
                    lit: true
                    litColor: Theme.sync
                    onClicked: newKit.create()
                }
            }
        }
    }

    FileDialog {
        id: sampleDialog
        property int instIndex: 0
        title: "Sound for " + (drums.fullNames[instIndex] || "")
        nameFilters: ["Audio files (*.wav *.flac *.mp3 *.ogg *.aif *.aiff *.m4a)", "All files (*)"]
        onAccepted: AppController.drumLoadUrl(instIndex, selectedFile)
    }
}
