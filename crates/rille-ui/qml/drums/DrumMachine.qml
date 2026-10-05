pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Dialogs
import QtQuick.Layouts
import rille.ui

// The drum machine: eight instruments, sixteen steps, always in time with
// the master clock. One sequencer row (the selected instrument) or four;
// the instrument tabs choose what the row and the knobs edit, and dragging
// them sets the track order the rows follow. Every control sends a `drum.*`
// target, so MIDI learn works on all of them.
Rectangle {
    id: drums
    readonly property var st: JSON.parse(AppController.drumsJson || "{}")
    readonly property var inst: st.inst || []
    readonly property int sel: st.selected || 0
    // Instruments left to right (and top to bottom in four rows).
    readonly property var order: st.order && st.order.length === 8 ? st.order : [0, 1, 2, 3, 4, 5, 6, 7]
    readonly property bool four: AppController.drumsRows === 4
    // Four rows: the half of the tracks the selected one is in.
    readonly property int pageStart: order.indexOf(sel) < 4 ? 0 : 4
    readonly property var hits: AppController.drumsHits.split(",")
    readonly property bool beatOn: ((AppController.clockBeat % 1) + 1) % 1 < 0.5
    readonly property bool external: AppController.mixerChannels.length > 0
    // Phone width: the steps get a row of their own (two lines of eight),
    // the sound, channel and kit go below them.
    readonly property bool narrow: width < 700
    // Mobile landscape: the steps beside the transport, sound, channel and
    // kit in a second row.
    readonly property bool twoRows: Theme.mobile && !narrow
    readonly property bool oneRow: !narrow && !twoRows
    // Wide screens: the layout stops growing here and sits in the middle,
    // so the steps keep their shape instead of stretching across the panel.
    readonly property int maxContentWidth: 1400
    readonly property var fullNames: ["Bass drum", "Snare drum", "Closed hi-hat", "Open hi-hat", "Clap", "Rim shot", "Tom", "Cymbal"]

    // Knob pages (see DrumKnobPage) of the selected instrument: its sound,
    // inserts, filter and sends.
    readonly property var soundPages: {
        var s = drums.inst[drums.sel] || {}
        var fx = s.fx || {}
        var v = (x, d) => x !== undefined ? x : d
        return [
            { name: "SRC", tip: "Pitch, length and level of the selected instrument", knobs: [
                { label: "TUNE", target: "drum.sel_tune", value: v(s.tune, 0.5), defaultValue: 0.5, bipolar: true,
                  format: x => { var st = Math.round((x - 0.5) * 24); return (st > 0 ? "+" : "") + st + " st" },
                  tip: "Pitch of the selected instrument, ±12 semitones" },
                { label: "DECAY", target: "drum.sel_decay", value: v(s.decay, 1), defaultValue: 1,
                  format: x => x >= 0.999 ? "FULL" : drums.percent(x),
                  tip: "Length of the selected instrument's sound: right = the whole sample, left = short" },
                { label: "LEVEL", target: "drum.sel_level", value: v(s.level, 1), defaultValue: 1, format: drums.percent,
                  tip: "Level of the selected instrument within the kit" }
            ] },
            { name: "FX", tip: "Bit reduction, sample-rate reduction and overdrive on the selected instrument", knobs: [
                { label: "BITS", target: "drum.sel_bits", value: v(fx.bits, 0), defaultValue: 0,
                  format: x => x <= 0 ? "OFF" : (16 - 14 * x).toFixed(1) + " bit",
                  tip: "Bit reduction: right = fewer bits, a cruder sound" },
                { label: "SRR", target: "drum.sel_srr", value: v(fx.srr, 0), defaultValue: 0,
                  format: x => x <= 0 ? "OFF" : "÷" + Math.pow(48, x).toFixed(1),
                  tip: "Sample-rate reduction: right = a lower rate, more aliasing" },
                { label: "DRIVE", target: "drum.sel_drive", value: v(fx.drive, 0), defaultValue: 0, format: drums.offPercent,
                  tip: "Overdrive on the selected instrument" }
            ] },
            { name: "FILT", tip: "Resonant filter on the selected instrument", knobs: [
                { label: "CUTOFF", target: "drum.sel_cutoff", value: v(fx.cutoff, 1), defaultValue: 1,
                  format: x => drums.hz(20 * Math.pow(1000, x)),
                  tip: "Filter cutoff, 20 Hz to 20 kHz" },
                { label: "RES", target: "drum.sel_res", value: v(fx.res, 0), defaultValue: 0, format: drums.percent,
                  tip: "Filter resonance" },
                { label: "TYPE", target: "drum.sel_ftype", value: v(fx.ftype, 0), defaultValue: 0,
                  format: x => ["LOW-PASS", "BAND-PASS", "HIGH-PASS"][Math.min(2, Math.floor(x * 3))],
                  tip: "Filter type: low-pass, band-pass or high-pass" }
            ] },
            { name: "SEND", tip: "How much of the selected instrument goes to the drums' delay and reverb", knobs: [
                { label: "DELAY", target: "drum.sel_delay", value: v(fx.delay, 0), defaultValue: 0, format: drums.offPercent,
                  tip: "Send to the drums' delay" },
                { label: "REVERB", target: "drum.sel_reverb", value: v(fx.reverb, 0), defaultValue: 0, format: drums.offPercent,
                  tip: "Send to the drums' reverb" }
            ] }
        ]
    }
    // Knob pages of the send effects and the master effects.
    readonly property var busPages: {
        var b = drums.st.bus || {}
        var v = (x, d) => x !== undefined ? x : d
        var times = ["1/32", "3/64", "1/16", "3/32", "1/8", "3/16", "1/4", "1/2", "1 BAR"]
        return [
            { name: "DLY", tip: "The drums' delay, in time with the master clock", knobs: [
                { label: "TIME", target: "drum.delay_time", value: v(b.delay_time, 0.5), defaultValue: 0.5,
                  format: x => times[Math.min(8, Math.floor(x * 9))], tip: "Delay time" },
                { label: "FDBK", target: "drum.delay_feedback", value: v(b.delay_feedback, 0.4), defaultValue: 0.4,
                  format: drums.percent, tip: "Delay feedback: how long the echoes go on" },
                { label: "FILTER", target: "drum.delay_filter", value: v(b.delay_filter, 0.5), defaultValue: 0.5, bipolar: true,
                  format: Theme.filterText, tip: "Filter on the echoes: left = low-pass, right = high-pass" }
            ] },
            { name: "REV", tip: "The drums' reverb", knobs: [
                { label: "SIZE", target: "drum.reverb_size", value: v(b.reverb_size, 0.5), defaultValue: 0.5,
                  format: x => (0.3 * Math.pow(40, x)).toFixed(1) + " s", tip: "Reverb decay time" },
                { label: "DAMP", target: "drum.reverb_damp", value: v(b.reverb_damp, 0.3), defaultValue: 0.3,
                  format: drums.percent, tip: "Reverb damping: right = darker" },
                { label: "PRE", target: "drum.reverb_predelay", value: v(b.reverb_predelay, 0.1), defaultValue: 0.1,
                  format: x => Math.round(x * 200) + " ms", tip: "Reverb pre-delay" }
            ] },
            { name: "COMP", tip: "Compressor on all the drums", knobs: [
                { label: "THRESH", target: "drum.comp_threshold", value: v(b.comp_threshold, 1), defaultValue: 1,
                  format: x => x >= 0.999 ? "OFF" : (-40 * (1 - x)).toFixed(1) + " dB",
                  tip: "Compressor threshold: right = off, left = more compression" },
                { label: "RATIO", target: "drum.comp_ratio", value: v(b.comp_ratio, 0.25), defaultValue: 0.25,
                  format: x => Math.pow(20, x).toFixed(1) + ":1", tip: "Compressor ratio" },
                { label: "REL", target: "drum.comp_release", value: v(b.comp_release, 0.4), defaultValue: 0.4,
                  format: x => Math.round(30 * Math.pow(1000 / 30, x)) + " ms", tip: "Compressor release" }
            ] },
            { name: "MST", tip: "Compressor sidechain and mix, and overdrive on all the drums", knobs: [
                { label: "SC", target: "drum.comp_sidechain", value: v(b.comp_sidechain, 0), defaultValue: 0,
                  format: x => { var i = Math.round(x * 8); return i === 0 ? "ALL" : drums.instName(i - 1) },
                  tip: "What the compressor listens to: all the drums, or one instrument (e.g. BD, to duck the rest under the kick)" },
                { label: "MIX", target: "drum.comp_mix", value: v(b.comp_mix, 1), defaultValue: 1, format: drums.percent,
                  tip: "Compressor mix: left = dry, right = fully compressed" },
                { label: "DRIVE", target: "drum.drive", value: v(b.drive, 0), defaultValue: 0, format: drums.offPercent,
                  tip: "Overdrive on all the drums, after the compressor" }
            ] }
        ]
    }

    function percent(x) {
        return Math.round(x * 100) + "%"
    }
    function offPercent(x) {
        return x <= 0 ? "OFF" : Math.round(x * 100) + "%"
    }
    function hz(f) {
        return f >= 1000 ? (f / 1000).toFixed(f >= 9950 ? 0 : 1) + " kHz" : Math.round(f) + " Hz"
    }

    function instName(i) {
        return drums.inst[i] ? drums.inst[i].name : ""
    }
    function instColor(i) {
        return drums.inst[i] ? drums.inst[i].color : Theme.sync
    }
    function loaded(i) {
        return drums.inst[i] ? drums.inst[i].loaded : true
    }
    // What instrument `i`'s sound is called, if known.
    function label(i) {
        return (drums.st.labels || [])[i] || ""
    }
    // Moves instrument `i`'s track `by` places.
    function moveTrack(i, by) {
        var from = drums.order.indexOf(i)
        var to = Math.max(0, Math.min(7, from + by))
        if (from >= 0 && to !== from)
            AppController.drumMoveTrack(from, to)
    }
    // Files or a library track dropped on instrument `i` (several files:
    // that track and the ones after it).
    function dropSounds(i, d) {
        if (d.hasUrls && d.urls.length > 0)
            AppController.drumLoadUrls(i, Array.prototype.map.call(d.urls, u => u.toString()).join("\n"))
        else if (d.getDataAsString("application/x-rille-track").length)
            AppController.drumLoadTrack(i, Number(d.getDataAsString("application/x-rille-track")))
        else if (d.getDataAsString("application/x-rille-path").length)
            AppController.drumLoadPath(i, Number(d.getDataAsString("application/x-rille-path")))
        d.acceptProposedAction()
    }
    // Saves the rack with the current kit; a factory kit under a new name.
    function saveRack() {
        if (drums.st.factory)
            newKit.open()
        else
            AppController.saveDrumRack()
    }
    function loadSounds(i) {
        sampleDialog.instIndex = i
        sampleDialog.open()
    }
    // Muted, or silent while another instrument is soloed.
    function silent(i) {
        var s = drums.inst[i]
        return !!s && (s.muted || (!!drums.st.solo && !s.soloed))
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

    // Heading of a section of controls.
    component SectionTitle: UiText {
        font.pixelSize: Theme.fontTiny
        font.bold: true
        font.letterSpacing: 0.5
    }
    // Hairline before a section; only when everything is in one row.
    component SectionRule: Rectangle {
        visible: drums.oneRow
        Layout.fillHeight: true
        Layout.topMargin: 4
        Layout.bottomMargin: 4
        implicitWidth: 1
        color: Theme.border
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
                    // Blinking while it waits for the bar to start or stop.
                    lit: AppController.drumsPlaying && (!(drums.st.waiting || drums.st.stopping) || drums.beatOn)
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
                DjButton {
                    flat: true
                    icon: "undo"
                    implicitWidth: 22
                    implicitHeight: 26
                    enabled: !!drums.st.undo
                    target: "drum.undo"
                    tip: "Undo the last pattern edit"
                }
                DjButton {
                    flat: true
                    icon: "redo"
                    implicitWidth: 22
                    implicitHeight: 26
                    enabled: !!drums.st.redo
                    target: "drum.redo"
                    tip: "Redo"
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
            // The tracks in order; drag one sideways to move it.
            Item {
                id: tabs
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.mobile ? 30 : 22
                Layout.fillHeight: false
                // Place a dragged tab would go to (−1: none dragged).
                property int dropAt: -1
                readonly property real slot: (width + tabRow.spacing) / 8
                function target(from, offset) {
                    return Math.max(0, Math.min(7, Math.round(from + offset / slot)))
                }
                RowLayout {
                    id: tabRow
                    anchors.fill: parent
                    spacing: 3
                    Repeater {
                        model: 8
                        DrumInstTab {
                            id: instTab
                            required property int index
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            inst: drums.order[index]
                            name: drums.instName(inst)
                            fullName: drums.fullNames[inst]
                            tint: drums.instColor(inst)
                            selected: drums.sel === inst
                            used: ((drums.st.rows || [])[inst] || ".").replace(/\./g, "").length > 0
                            muted: drums.silent(inst)
                            loaded: drums.loaded(inst)
                            label: drums.label(inst)
                            hits: drums.hits[inst] || ""
                            onMenuRequested: instMenu.openFor(inst)
                            onDragMoved: tabs.dropAt = tabs.target(index, dragOffset)
                            onDragDropped: {
                                var to = tabs.target(index, dragOffset)
                                tabs.dropAt = -1
                                if (to !== index)
                                    AppController.drumMoveTrack(index, to)
                            }
                            onDraggingChanged: if (!dragging) tabs.dropAt = -1
                        }
                    }
                }
                // Where the dragged track lands.
                Rectangle {
                    visible: tabs.dropAt >= 0
                    x: tabs.dropAt * tabs.slot - 1
                    width: tabs.slot - tabRow.spacing + 2
                    height: parent.height
                    radius: 3
                    color: "transparent"
                    border.color: Theme.sync
                    border.width: 2
                }
            }
            Repeater {
                model: drums.four ? 4 : 1
                RowLayout {
                    id: seqRow
                    required property int index
                    readonly property int instIndex: drums.four ? drums.order[drums.pageStart + index] : drums.sel
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: 6
                    // Four rows: name and mute of each.
                    Rectangle {
                        visible: drums.four
                        Layout.preferredWidth: 80
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
                            DjButton {
                                text: "S"
                                implicitWidth: 22
                                implicitHeight: 20
                                fontSize: Theme.fontTiny
                                target: "drum.inst_solo." + (seqRow.instIndex + 1)
                                lit: drums.inst[seqRow.instIndex] ? drums.inst[seqRow.instIndex].soloed : false
                                litColor: Theme.sync
                                tip: "Solo " + drums.fullNames[seqRow.instIndex] + ": only soloed instruments play"
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
        RowLayout {
            Layout.row: drums.narrow ? 2 : (drums.twoRows ? 1 : 0)
            Layout.column: drums.narrow || drums.twoRows ? 0 : 4
            Layout.alignment: Qt.AlignVCenter
            Layout.fillWidth: false
            Layout.fillHeight: drums.oneRow
            spacing: 8
            SectionRule {}
            ColumnLayout {
                Layout.alignment: Qt.AlignVCenter
                spacing: 0
                RowLayout {
                    Layout.preferredHeight: 20
                    spacing: 4
                    SectionTitle {
                        text: (drums.fullNames[drums.sel] || "").toUpperCase()
                        color: drums.instColor(drums.sel)
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
                    DjButton {
                        text: "S"
                        implicitWidth: 22
                        implicitHeight: 18
                        fontSize: Theme.fontTiny
                        target: "drum.inst_solo." + (drums.sel + 1)
                        lit: drums.inst[drums.sel] ? drums.inst[drums.sel].soloed : false
                        litColor: Theme.sync
                        tip: "Solo the selected instrument: only soloed instruments play"
                    }
                }
                DrumKnobPage {
                    pages: drums.soundPages
                    color: drums.instColor(drums.sel)
                }
            }
        }

        // --- The drums' send and master effects -----------------------------
        RowLayout {
            Layout.row: drums.narrow ? 3 : (drums.twoRows ? 1 : 0)
            Layout.column: drums.narrow ? 0 : (drums.twoRows ? 1 : 5)
            Layout.columnSpan: drums.narrow ? 3 : 1
            Layout.alignment: Qt.AlignVCenter
            Layout.fillWidth: false
            Layout.fillHeight: drums.oneRow
            spacing: 8
            SectionRule {}
            ColumnLayout {
                Layout.alignment: Qt.AlignVCenter
                spacing: 0
                RowLayout {
                    Layout.preferredHeight: 20
                    SectionTitle {
                        text: "DRUM FX"
                        color: Theme.textDim
                    }
                }
                DrumKnobPage {
                    pages: drums.busPages
                    color: Theme.fx
                }
            }
        }

        // --- The drums' channel: filter, FX, cue and the volume fader --------
        RowLayout {
            Layout.row: drums.narrow ? 2 : (drums.twoRows ? 1 : 0)
            Layout.column: drums.narrow ? 1 : (drums.twoRows ? 2 : 6)
            Layout.columnSpan: drums.narrow ? 2 : 1
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredHeight: drums.oneRow ? -1 : 70
            Layout.fillWidth: false
            Layout.fillHeight: drums.oneRow
            spacing: 8
            SectionRule {}
            ColumnLayout {
                Layout.alignment: Qt.AlignVCenter
                spacing: 0
                RowLayout {
                    Layout.preferredHeight: 20
                    spacing: 4
                    SectionTitle {
                        text: "OUTPUT"
                        color: Theme.textDim
                    }
                    Item { Layout.fillWidth: true }
                    DjButton {
                        icon: "headphones"
                        implicitWidth: 30
                        implicitHeight: 18
                        target: "drum.pfl"
                        lit: !!drums.st.pfl
                        litColor: Theme.sync
                        tip: "Drums in the headphones"
                    }
                }
                RowLayout {
                    spacing: 4
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
                    ColumnLayout {
                        Layout.alignment: Qt.AlignTop
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
                    }
                }
            }
            // The drums' volume in the main mix: a fader beside the meter,
            // like a mixer channel.
            Fader {
                Layout.fillHeight: true
                Layout.preferredWidth: 26
                ticks: 6
                value: drums.st.level !== undefined ? drums.st.level : 0.8
                defaultValue: 0.8
                target: "drum.level"
                color: Theme.text
                tip: "Drum volume in the main mix (not on the crossfader): " + Math.round((drums.st.level !== undefined ? drums.st.level : 0.8) * 100) + "%"
            }
            VuMeter {
                Layout.fillHeight: true
                Layout.preferredWidth: 5
                Layout.leftMargin: -5
                Layout.topMargin: 4
                Layout.bottomMargin: 4
                level: AppController.drumsMeter
                segments: 14
            }
        }

        Item {
            visible: drums.narrow
            Layout.row: 5
            Layout.column: 0
            Layout.fillHeight: true
        }

        // --- Kit ---------------------------------------------------------------
        RowLayout {
            Layout.row: drums.narrow ? 4 : (drums.twoRows ? 1 : 0)
            Layout.column: drums.narrow ? 0 : (drums.twoRows ? 3 : 7)
            Layout.columnSpan: drums.narrow ? 3 : 1
            Layout.alignment: drums.twoRows ? Qt.AlignVCenter | Qt.AlignRight : Qt.AlignVCenter
            Layout.fillWidth: drums.narrow
            Layout.fillHeight: drums.oneRow
            spacing: 8
            SectionRule {}
            ColumnLayout {
                Layout.alignment: Qt.AlignVCenter
                Layout.fillWidth: drums.narrow
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
            text: drums.fullNames[instMenu.instIndex] + " · " + (drums.label(instMenu.instIndex) || drums.st.kit || "")
        }
        StyledMenuItem {
            iconName: "play"
            text: "Play"
            onTriggered: drums.tap("drum.trigger." + (instMenu.instIndex + 1))
        }
        StyledMenuItem {
            iconName: "import"
            text: "Load a sound…"
            onTriggered: drums.loadSounds(instMenu.instIndex)
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
            iconName: "chevron-left"
            enabled: drums.order.indexOf(instMenu.instIndex) > 0
            text: "Move left"
            onTriggered: drums.moveTrack(instMenu.instIndex, -1)
        }
        StyledMenuItem {
            iconName: "chevron-right"
            enabled: drums.order.indexOf(instMenu.instIndex) < 7
            text: "Move right"
            onTriggered: drums.moveTrack(instMenu.instIndex, 1)
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
            text: "Rack: " + (drums.st.kit || "") + (drums.st.factory ? " (factory)" : "")
        }
        StyledMenuItem {
            iconName: "drums"
            text: "Sounds…"
            onTriggered: rackPopup.open()
        }
        StyledMenuItem {
            iconName: "import"
            text: "Load sounds…"
            onTriggered: drums.loadSounds(-1)
        }
        StyledMenuItem {
            iconName: "import"
            text: "Load a sound into " + drums.instName(drums.sel) + "…"
            onTriggered: drums.loadSounds(drums.sel)
        }
        StyledMenuItem {
            iconName: "music"
            text: "Load the selected track into " + drums.instName(drums.sel)
            onTriggered: AppController.drumLoadSelected(drums.sel)
        }
        StyledMenuSeparator {}
        StyledMenuItem {
            iconName: "check"
            text: drums.st.factory ? "Save rack…" : "Save rack"
            onTriggered: drums.saveRack()
        }
        StyledMenuItem {
            iconName: "plus"
            text: "Save rack as…"
            onTriggered: newKit.open()
        }
        StyledMenuItem {
            iconName: "external"
            text: "Export rack…"
            onTriggered: exportDialog.open()
        }
        StyledMenuItem {
            iconName: "import"
            text: "Import rack…"
            onTriggered: importDialog.open()
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
                text: "Save rack as"
                font.pixelSize: Theme.fontLarge
                font.bold: true
            }
            UiText {
                Layout.preferredWidth: 320
                text: "The sounds of " + (drums.st.kit || "") + ", the track order and each instrument's tune, decay and level go to a kit folder of its own."
                color: Theme.textDim
                wrapMode: Text.WordWrap
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
                    text: "Save"
                    lit: true
                    litColor: Theme.sync
                    onClicked: newKit.create()
                }
            }
        }
    }

    // The rack: which sound each track plays, in the track order.
    Popup {
        id: rackPopup
        anchors.centerIn: Overlay.overlay
        width: Math.min(470, Overlay.overlay ? Overlay.overlay.width - 32 : 470)
        modal: true
        padding: 14
        background: Rectangle { color: Theme.panelRaised; border.color: Theme.border; radius: 8 }
        ColumnLayout {
            width: parent.width
            spacing: 6
            RowLayout {
                Layout.fillWidth: true
                UiText {
                    Layout.fillWidth: true
                    text: "Sounds · " + (drums.st.kit || "")
                    font.pixelSize: Theme.fontLarge
                    font.bold: true
                    elide: Text.ElideRight
                }
                DjButton {
                    flat: true
                    icon: "x"
                    implicitWidth: 26
                    implicitHeight: 24
                    onClicked: rackPopup.close()
                }
            }
            UiText {
                Layout.fillWidth: true
                text: "Drop files on a track, or load several at once: kick.wav, snare.wav … go to their tracks, the others to the free tracks from the left."
                color: Theme.textDim
                wrapMode: Text.WordWrap
            }
            Repeater {
                model: 8
                Rectangle {
                    id: rackRow
                    required property int index
                    readonly property int instIndex: drums.order[index]
                    readonly property bool loaded: drums.loaded(instIndex)
                    Layout.fillWidth: true
                    implicitHeight: 30
                    radius: 3
                    color: rowDrop.containsDrag ? Theme.controlHover : Theme.control
                    border.color: rowDrop.containsDrag ? Theme.sync : (drums.sel === instIndex ? drums.instColor(instIndex) : Theme.border)
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 4
                        spacing: 6
                        UiText {
                            Layout.preferredWidth: 26
                            text: drums.instName(rackRow.instIndex)
                            color: drums.instColor(rackRow.instIndex)
                            font.pixelSize: Theme.fontSmall
                            font.bold: true
                        }
                        UiText {
                            Layout.fillWidth: true
                            text: rackRow.loaded ? (drums.label(rackRow.instIndex) || drums.fullNames[rackRow.instIndex]) : "No sound"
                            color: rackRow.loaded ? Theme.text : Theme.textFaint
                            elide: Text.ElideRight
                        }
                        DjButton {
                            flat: true
                            icon: "play"
                            implicitWidth: 26
                            implicitHeight: 22
                            enabled: rackRow.loaded
                            target: "drum.trigger." + (rackRow.instIndex + 1)
                            tip: "Play"
                        }
                        DjButton {
                            flat: true
                            icon: "import"
                            implicitWidth: 26
                            implicitHeight: 22
                            tip: "Load a sound into " + drums.fullNames[rackRow.instIndex]
                            onClicked: drums.loadSounds(rackRow.instIndex)
                        }
                        DjButton {
                            flat: true
                            icon: "trash"
                            implicitWidth: 26
                            implicitHeight: 22
                            enabled: !drums.st.factory && rackRow.loaded
                            tip: "Remove the sound"
                            onClicked: AppController.drumClearSample(rackRow.instIndex)
                        }
                    }
                    DropArea {
                        id: rowDrop
                        anchors.fill: parent
                        keys: ["application/x-rille-track", "application/x-rille-path", "text/uri-list"]
                        onDropped: d => drums.dropSounds(rackRow.instIndex, d)
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 4
                spacing: 4
                DjButton {
                    icon: "import"
                    text: "Load sounds…"
                    implicitHeight: 26
                    onClicked: drums.loadSounds(-1)
                }
                Item { Layout.fillWidth: true }
                DjButton {
                    text: "Save rack"
                    implicitHeight: 26
                    tip: drums.st.factory ? "Factory kits stay as they are: saves it under a name of your own" : "Saves the track order and each instrument's tune, decay and level with " + (drums.st.kit || "")
                    onClicked: drums.saveRack()
                }
                DjButton {
                    text: "Export…"
                    implicitHeight: 26
                    tip: "Writes the rack (sounds as WAV and a kit.toml) into a new folder"
                    onClicked: exportDialog.open()
                }
            }
        }
    }

    FileDialog {
        id: sampleDialog
        // −1: the whole rack, each file to the track its name suggests.
        property int instIndex: 0
        title: instIndex < 0 ? "Sounds for the rack" : "Sound for " + (drums.fullNames[instIndex] || "") + " (several: this track and the ones after it)"
        fileMode: FileDialog.OpenFiles
        nameFilters: ["Audio files (*.wav *.flac *.mp3 *.ogg *.aif *.aiff *.m4a)", "All files (*)"]
        onAccepted: AppController.drumLoadUrls(instIndex, Array.prototype.map.call(selectedFiles, u => u.toString()).join("\n"))
    }
    FolderDialog {
        id: exportDialog
        title: "Export the rack into"
        onAccepted: AppController.exportDrumRack(selectedFolder)
    }
    FolderDialog {
        id: importDialog
        title: "Import a rack (a folder of sounds)"
        onAccepted: AppController.importDrumRack(selectedFolder)
    }
}
