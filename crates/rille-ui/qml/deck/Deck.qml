pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import rille.ui

// One track deck. `dc` is its DeckController.
Rectangle {
    id: deck
    required property DeckController dc
    /// 4-deck layout: shorter header, transport and overview; the hotcue
    /// row only on tall windows, so the waveform keeps some height.
    property bool compact: false
    property bool gridEditing: false
    readonly property string letter: String.fromCharCode(65 + dc.deck)
    readonly property var hotcues: JSON.parse(dc.hotcuesJson || "[]")
    readonly property color gridColor: !dc.hasGrid ? Theme.textFaint : (dc.gridText.length ? Theme.warn : Theme.play)

    function t(name) {
        return dc.target(name)
    }
    function tap(name) {
        AppController.press(t(name), true)
        AppController.press(t(name), false)
    }
    // Hover text of the grid badge: the state, what each note means, and
    // what to do about it.
    readonly property var gridNoteHelp: ({
        "variable tempo": "variable tempo: the tempo drifts, the grid follows it",
        "tempo change": "tempo change: the track changes its tempo",
        "check tempo": "check tempo: the BPM may be half or double (×2 / ÷2 in the grid editor)",
        "check bar start": "check bar start: beat 1 of the bar may be off (BAR START on the real downbeat)",
        "no clear beat": "no clear beat: no steady rhythm found"
    })
    function gridTip() {
        if (!dc.hasGrid)
            return dc.analyzing ? "Beatgrid: analyzing…" : "No beatgrid yet: the track has not been analyzed."
        var lines = [dc.gridLocked ? "Beatgrid locked: re-analysis keeps it." : (dc.gridText.length ? "Beatgrid needs a check:" : "Beatgrid OK.")]
        var notes = dc.gridText.length ? dc.gridText.split(", ") : []
        for (var i = 0; i < notes.length; i++)
            lines.push("•  " + (gridNoteHelp[notes[i]] || notes[i]))
        lines.push("Click to open the grid editor.")
        return lines.join("\n")
    }

    color: dropArea.containsDrag ? Theme.panelRaised : Theme.panel
    radius: Theme.radius
    clip: true
    // Narrow decks (small windows) hide the less important buttons.
    readonly property bool roomy: width > 600
    readonly property bool medium: width > 530
    readonly property int transportHeight: compact ? 28 : 34
    readonly property bool showHotcues: !compact || height >= 240
    border.color: dropArea.containsDrag ? Theme.sync : Theme.panelEdge

    DropArea {
        id: dropArea
        anchors.fill: parent
        keys: ["application/x-rille-track", "application/x-rille-path", "application/x-rille-beatport", "text/uri-list"]
        onDropped: drop => {
            if (drop.hasUrls && drop.urls.length > 0)
                AppController.loadUrl(deck.dc.deck, drop.urls[0])
            else if (drop.getDataAsString("application/x-rille-track").length)
                AppController.loadTrack(deck.dc.deck, Number(drop.getDataAsString("application/x-rille-track")))
            else if (drop.getDataAsString("application/x-rille-beatport").length)
                AppController.loadBeatport(deck.dc.deck, Number(drop.getDataAsString("application/x-rille-beatport")))
            else if (drop.getDataAsString("application/x-rille-path").length)
                AppController.loadPath(deck.dc.deck, Number(drop.getDataAsString("application/x-rille-path")))
            drop.acceptProposedAction()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 6
        spacing: 4

        // --- Track header -------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: deck.compact ? 40 : 50
            spacing: 8
            Rectangle {
                Layout.preferredWidth: deck.compact ? 40 : 50
                Layout.preferredHeight: deck.compact ? 40 : 50
                color: Theme.control
                radius: 3
                clip: true
                Image {
                    anchors.fill: parent
                    source: deck.dc.cover
                    fillMode: Image.PreserveAspectCrop
                    visible: deck.dc.cover.length > 0
                    asynchronous: true
                }
                Icon {
                    anchors.centerIn: parent
                    visible: deck.dc.cover.length === 0
                    name: "music"
                    size: 22
                    color: Theme.textFaint
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                UiText {
                    Layout.fillWidth: true
                    text: deck.dc.loaded || deck.dc.loading ? deck.dc.title : "Drop a track here"
                    color: deck.dc.loaded ? Theme.text : Theme.textFaint
                    font.pixelSize: Theme.fontLarge
                    font.bold: true
                    elide: Text.ElideRight
                }
                UiText {
                    Layout.fillWidth: true
                    text: deck.dc.error.length ? deck.dc.error : (deck.dc.artist + (deck.dc.info.length ? "  ·  " + deck.dc.info : ""))
                    color: deck.dc.error.length ? Theme.danger : Theme.textDim
                    elide: Text.ElideRight
                }
            }
            ColumnLayout {
                spacing: -2
                UiText {
                    Layout.alignment: Qt.AlignRight
                    text: deck.dc.loaded ? "−" + Theme.formatTime(deck.dc.duration - deck.dc.position) : "—"
                    color: deck.dc.endWarning ? Theme.danger : Theme.text
                    font.pixelSize: deck.compact ? 20 : Theme.fontHuge
                    font.family: Theme.fontMono
                    font.weight: Font.Medium
                    font.letterSpacing: -1
                }
                UiText {
                    Layout.alignment: Qt.AlignRight
                    text: deck.dc.loaded ? Theme.formatTime(deck.dc.position) + " / " + Theme.formatTime(deck.dc.duration) : ""
                    color: Theme.textDim
                    font.pixelSize: Theme.fontSmall
                }
            }
            ColumnLayout {
                spacing: -2
                Layout.preferredWidth: 86
                UiText {
                    Layout.alignment: Qt.AlignRight
                    text: deck.dc.loaded && deck.dc.bpm > 0 ? deck.dc.bpm.toFixed(2) : "—"
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
            Rectangle {
                Layout.preferredWidth: 42
                Layout.preferredHeight: 24
                radius: 3
                color: deck.dc.keyText.length ? Qt.darker(deck.dc.keyColor, 2.4) : Theme.control
                border.color: deck.dc.keyText.length ? deck.dc.keyColor : Theme.border
                MouseArea { id: keyArea; anchors.fill: parent; hoverEnabled: true }
                Tip { text: "Musical key of the track"; visible: keyArea.containsMouse }
                UiText {
                    anchors.centerIn: parent
                    text: deck.dc.keyText.length ? deck.dc.keyText : "—"
                    color: deck.dc.keyText.length ? deck.dc.keyColor : Theme.textDim
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

        // --- Sync row -----------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            spacing: 4
            DjButton { text: "SYNC"; target: deck.t("sync"); lit: deck.dc.sync; litColor: Theme.sync; implicitHeight: 24; tip: "Sync: follow the tempo and beat phase of the master" }
            DjButton { text: "MASTER"; target: deck.t("master"); lit: deck.dc.master; litColor: Theme.accent; implicitHeight: 24; tip: "Master: this deck sets the tempo the others sync to" }
            // Phase to the master (±½ beat) above a 4-beat bar counter.
            Item {
                Layout.fillWidth: true
                Layout.minimumWidth: 40
                Layout.preferredHeight: 24
                MouseArea { id: phaseArea; anchors.fill: parent; hoverEnabled: true }
                Tip {
                    text: "Phase against the master: centred when the beats line up, left = behind, right = ahead.\nBelow: the beat in the bar (1 lights up amber)."
                    visible: phaseArea.containsMouse
                }
                Rectangle {
                    id: phase
                    width: parent.width
                    height: 10
                    color: Theme.bg
                    radius: 2
                    border.color: Theme.border
                    Rectangle { anchors.centerIn: parent; width: 1; height: parent.height; color: Theme.textFaint }
                    // Phase against the master: only meaningful while this deck
                    // plays and another deck leads the tempo.
                    Rectangle {
                        visible: deck.dc.loaded && deck.dc.hasGrid && deck.dc.playing
                            && AppController.masterDeck >= 0 && AppController.masterDeck !== deck.dc.deck
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
                            color: deck.dc.beatInBar === index ? (index === 0 ? Theme.accent : Theme.text) : Theme.meterOff
                        }
                    }
                }
            }
            // Grid state as an icon: green check when clean, amber alert with
            // notes to check, lock when locked; the details on hover.
            Rectangle {
                visible: deck.dc.loaded
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
                radius: 12
                color: gridArea.containsMouse ? Qt.darker(deck.gridColor, 2.2) : Qt.darker(deck.gridColor, 3)
                border.color: deck.gridColor
                Icon {
                    anchors.centerIn: parent
                    name: deck.dc.gridLocked ? "lock" : (deck.dc.gridText.length && deck.dc.hasGrid ? "alert" : (deck.dc.hasGrid ? "check" : (deck.dc.analyzing ? "analyze" : "grid")))
                    size: 14
                    color: deck.gridColor
                }
                MouseArea {
                    id: gridArea
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: deck.gridEditing = !deck.gridEditing
                }
                Tip {
                    text: deck.gridTip()
                    visible: gridArea.containsMouse && !gridArea.pressed
                }
            }
            DjButton { visible: deck.medium; icon: "chevrons-left"; target: deck.t("beatjump_back"); implicitWidth: 28; implicitHeight: 24; tip: "Beat jump back by the loop size" }
            DjButton { visible: deck.medium; icon: "chevrons-right"; target: deck.t("beatjump_forward"); implicitWidth: 28; implicitHeight: 24; tip: "Beat jump forward by the loop size" }
            DjButton { icon: "lock"; text: "KEY"; target: deck.t("keylock"); lit: deck.dc.keylock; litColor: Theme.sync; implicitHeight: 24; tip: "Keylock: the pitch stays the same when the tempo changes" }
            DjButton { icon: "metronome"; target: deck.t("tick"); lit: deck.dc.tick; litColor: Theme.warn; implicitWidth: 28; implicitHeight: 24; tip: "Metronome: click on the beatgrid's beats, to check the grid by ear" }
            DjButton { visible: deck.roomy; implicitHeight: 24; icon: "zoom-out"; implicitWidth: 30; tip: "Zoom the waveform out (or mouse wheel on it)"; onClicked: wave.seconds = Math.min(32, wave.seconds * 1.25) }
            DjButton { visible: deck.roomy; implicitHeight: 24; icon: "zoom-in"; implicitWidth: 28; tip: "Zoom the waveform in (or mouse wheel on it)"; onClicked: wave.seconds = Math.max(2, wave.seconds * 0.8) }
            DjButton { implicitHeight: 24; icon: "grid"; lit: deck.gridEditing; litColor: Theme.warn; tip: "Grid editor: correct the beatgrid (in place of the hotcues)"; onClicked: deck.gridEditing = !deck.gridEditing }
            DjButton { implicitHeight: 24; icon: "eject"; implicitWidth: 30; tip: "Eject the track"; onClicked: AppController.eject(deck.dc.deck) }
        }

        // --- Waveform + tempo fader ----------------------------------------
        // --- Waveform and overview, tempo fader beside both ---------------------
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 4
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 3
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: Theme.bg
                    radius: 3
                    border.color: Theme.border
                    clip: true
                    WaveformView {
                        id: wave
                        anchors.fill: parent // qmllint disable unqualified
                        anchors.margins: 1
                        deck: deck.dc.deck
                        seconds: 8
                    }
                    FrameAnimation {
                        running: deck.visible
                        onTriggered: wave.update()
                    }
                    // Drag on the waveform to scratch; the wheel zooms.
                    MouseArea {
                        anchors.fill: parent
                        property real lastX: 0
                        onPressed: mouse => {
                            lastX = mouse.x
                            AppController.press(deck.t("jog_touch"), true)
                        }
                        onPositionChanged: mouse => {
                            var secs = (lastX - mouse.x) * wave.seconds / width
                            AppController.nudge(deck.t("jog"), secs / 1.8)
                            lastX = mouse.x
                        }
                        onReleased: AppController.press(deck.t("jog_touch"), false)
                        onWheel: wheel => wave.seconds = Math.max(2, Math.min(32, wave.seconds * (wheel.angleDelta.y > 0 ? 0.8 : 1.25)))
                    }
                    UiText {
                        anchors.centerIn: parent
                        visible: !deck.dc.loading && !deck.dc.loaded
                        text: "Drag a track from the browser"
                        color: Theme.textFaint
                    }
                    // Loading: a streamed track downloads first (progress,
                    // speed, time left), then every track is decoded.
                    Item {
                        id: loadingCard
                        anchors.fill: parent
                        anchors.margins: 1
                        visible: deck.dc.loading
                        z: 10
                        readonly property real progress: deck.dc.downloadProgress
                        readonly property bool determinate: progress > 0
                        // A card in the middle: the previous track's waveform
                        // (which may still be playing) stays visible around it.
                        Rectangle {
                            anchors.centerIn: card
                            width: card.width + 32
                            height: card.height + (deck.compact ? 12 : 22)
                            radius: 6
                            color: Theme.panel
                            border.color: Theme.border
                        }
                        Column {
                            id: card
                            anchors.centerIn: parent
                            width: Math.min(parent.width - 72, 300)
                            spacing: deck.compact ? 4 : 7
                            Row {
                                anchors.horizontalCenter: parent.horizontalCenter
                                spacing: 7
                                Icon {
                                    anchors.verticalCenter: parent.verticalCenter
                                    name: loadingCard.progress >= 0 ? "cloud" : "analyze"
                                    size: 15
                                    color: Theme.sync
                                    SequentialAnimation on opacity {
                                        running: loadingCard.visible
                                        loops: Animation.Infinite
                                        NumberAnimation { from: 1; to: 0.35; duration: 700; easing.type: Easing.InOutSine }
                                        NumberAnimation { from: 0.35; to: 1; duration: 700; easing.type: Easing.InOutSine }
                                    }
                                }
                                UiText {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: deck.dc.loadingText
                                    font.bold: true
                                    font.pixelSize: Theme.fontSmall
                                }
                            }
                            Rectangle {
                                id: bar
                                width: parent.width
                                height: 4
                                radius: 2
                                color: Theme.control
                                clip: true
                                // How much is downloaded, with a light running over it.
                                Rectangle {
                                    visible: loadingCard.determinate
                                    width: bar.width * Math.min(1, loadingCard.progress)
                                    height: bar.height
                                    radius: 2
                                    color: Theme.sync
                                    clip: true
                                    Behavior on width { NumberAnimation { duration: 200 } }
                                    Rectangle {
                                        width: 60
                                        height: parent.height
                                        opacity: 0.55
                                        gradient: Gradient {
                                            orientation: Gradient.Horizontal
                                            GradientStop { position: 0; color: "transparent" }
                                            GradientStop { position: 0.5; color: Theme.text }
                                            GradientStop { position: 1; color: "transparent" }
                                        }
                                        NumberAnimation on x {
                                            running: loadingCard.visible && loadingCard.determinate
                                            loops: Animation.Infinite
                                            from: -60
                                            to: bar.width
                                            duration: 1400
                                        }
                                    }
                                }
                                // Before the size is known, and while decoding: a sweep.
                                Rectangle {
                                    visible: !loadingCard.determinate
                                    width: bar.width * 0.3
                                    height: bar.height
                                    radius: 2
                                    gradient: Gradient {
                                        orientation: Gradient.Horizontal
                                        GradientStop { position: 0; color: "transparent" }
                                        GradientStop { position: 0.5; color: Theme.sync }
                                        GradientStop { position: 1; color: "transparent" }
                                    }
                                    NumberAnimation on x {
                                        running: loadingCard.visible && !loadingCard.determinate
                                        loops: Animation.Infinite
                                        from: -bar.width * 0.3
                                        to: bar.width
                                        duration: 1100
                                        easing.type: Easing.InOutQuad
                                    }
                                }
                            }
                            UiText {
                                anchors.horizontalCenter: parent.horizontalCenter
                                visible: text.length > 0
                                text: deck.dc.downloadText
                                color: Theme.textDim
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.fontSmall
                            }
                        }
                    }
                }
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: deck.compact ? 22 : 30
                    color: Theme.bg
                    radius: 3
                    border.color: deck.dc.endWarning && blink.on ? Theme.danger : Theme.border
                    clip: true
                    WaveformView {
                        id: overview
                        anchors.fill: parent // qmllint disable unqualified
                        anchors.margins: 1
                        deck: deck.dc.deck
                        overview: true
                    }
                    Timer {
                        interval: 50
                        running: deck.visible
                        repeat: true
                        onTriggered: overview.update()
                    }
                    Timer {
                        id: blink
                        property bool on: false
                        interval: 400
                        running: deck.dc.endWarning
                        repeat: true
                        onTriggered: on = !on
                        onRunningChanged: on = false
                    }
                    // Numbered hotcue flags.
                    Repeater {
                        model: deck.hotcues
                        Rectangle {
                            required property var modelData
                            visible: deck.dc.duration > 0
                            x: 1 + modelData.secs / Math.max(1, deck.dc.duration) * (overview.width) - 1
                            y: 1
                            width: 12
                            height: 11
                            color: modelData.color
                            UiText { anchors.centerIn: parent; text: parent.modelData.slot + 1; color: Theme.textOnLit; font.pixelSize: 9; font.bold: true }
                        }
                    }
                    MouseArea {
                        anchors.fill: parent
                        onPressed: mouse => AppController.setValue(deck.t("seek"), mouse.x / width)
                    }
                }
            }
            ColumnLayout {
                Layout.fillHeight: true
                spacing: 2
                Fader {
                    Layout.fillHeight: true
                    Layout.preferredWidth: 26
                    value: deck.dc.tempoFader
                    target: deck.t("tempo")
                    centered: true
                    ticks: 9
                    color: Math.abs(deck.dc.tempoFader - 0.5) < 0.001 ? Theme.textDim : Theme.warn
                    tip: "Tempo ±" + Math.round(AppController.tempoRange * 100) + " % (range in Settings); double-click resets"
                }
                UiText {
                    Layout.alignment: Qt.AlignHCenter
                    text: "±" + Math.round(AppController.tempoRange * 100)
                    color: Theme.textFaint
                    font.pixelSize: Theme.fontTiny
                }
            }
        }

        // --- Transport and loops -----------------------------------------------
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
                tip: "Play / pause"
            }
            DjButton { flat: true; icon: "skip-back"; target: deck.t("jump_start"); implicitWidth: 40; implicitHeight: deck.transportHeight; tip: "Back to the start of the track (Shift+CUE key); a stopped deck stays stopped" }
            DjButton { flat: true; text: "CUE"; target: deck.t("cue"); lit: deck.dc.cueHeld; implicitHeight: deck.transportHeight; fontSize: Theme.fontLarge; tip: "Cue: while stopped, set the cue point and play while held; while playing, back to the cue point" }
            DjButton { flat: true; text: "CUP"; target: deck.t("cup"); implicitHeight: deck.transportHeight; fontSize: Theme.fontLarge; tip: "Cue-play: back to the cue point, plays on release" }
            DjButton { visible: deck.medium; flat: true; text: "FLX"; target: deck.t("flux"); lit: deck.dc.flux; litColor: Theme.sync; implicitHeight: deck.transportHeight; fontSize: Theme.fontLarge; tip: "Flux: after a loop, jump or reverse, play on from where the track would be now" }
            DjButton { visible: deck.medium; flat: true; text: "REV"; target: deck.t("reverse"); lit: deck.dc.reverse; litColor: Theme.danger; implicitHeight: deck.transportHeight; fontSize: Theme.fontLarge; tip: "Reverse: play backwards" }
            Item { Layout.fillWidth: true }
            LoopSelector { dc: deck.dc; implicitHeight: deck.transportHeight }
            DjButton { flat: true; text: "IN"; target: deck.t("loop_in"); implicitHeight: deck.transportHeight; fontSize: Theme.fontLarge; tip: "Loop in: set the loop start here" }
            DjButton { flat: true; text: "OUT"; target: deck.t("loop_out"); implicitHeight: deck.transportHeight; fontSize: Theme.fontLarge; tip: "Loop out: set the loop end here and start the loop" }
            DjButton { visible: deck.roomy; flat: true; icon: "loop"; lit: deck.dc.loopActive; litColor: Theme.sync; implicitHeight: deck.transportHeight; tip: "Loop on / off"; onClicked: deck.tap("loop_toggle") }
        }

        // --- Grid editor -----------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            visible: deck.gridEditing
            Layout.preferredHeight: 26
            spacing: 3
            UiText { text: "GRID"; color: Theme.warn; font.pixelSize: Theme.fontSmall; font.bold: true }
            DjButton { implicitHeight: 26; icon: "chevrons-left"; tip: "Move the grid 10 ms earlier"; onClicked: AppController.gridEdit(deck.dc.deck, "move", -10) }
            DjButton { implicitHeight: 26; icon: "chevron-left"; tip: "Move the grid 1 ms earlier"; onClicked: AppController.gridEdit(deck.dc.deck, "move", -1) }
            DjButton { implicitHeight: 26; icon: "chevron-right"; tip: "Move the grid 1 ms later"; onClicked: AppController.gridEdit(deck.dc.deck, "move", 1) }
            DjButton { implicitHeight: 26; icon: "chevrons-right"; tip: "Move the grid 10 ms later"; onClicked: AppController.gridEdit(deck.dc.deck, "move", 10) }
            DjButton { implicitHeight: 26; text: "×2"; tip: "Double the BPM"; onClicked: AppController.gridEdit(deck.dc.deck, "double", 0) }
            DjButton { implicitHeight: 26; text: "÷2"; tip: "Halve the BPM"; onClicked: AppController.gridEdit(deck.dc.deck, "halve", 0) }
            DjButton { implicitHeight: 26; text: "BEAT HERE"; tip: "Move the grid so the nearest beat sits on the play position"; onClicked: AppController.gridEdit(deck.dc.deck, "beat", 0) }
            DjButton { implicitHeight: 26; text: "BAR START"; tip: "Make the beat nearest to the play position beat 1 of the bar"; onClicked: AppController.gridEdit(deck.dc.deck, "downbeat", 0) }
            DjButton { implicitHeight: 26; text: "GRID START"; tip: "Start the grid here: the kick at the play position becomes beat 1 of a bar (move the play position onto the first kick first)"; onClicked: AppController.gridEdit(deck.dc.deck, "barstart", 0) }
            DjButton { implicitHeight: 26; text: "TAP"; tip: "Tap along with the beat; four or more taps set the BPM"; onClicked: AppController.gridEdit(deck.dc.deck, "tap", 0) }
            DjButton { implicitHeight: 26; icon: "lock"; lit: deck.dc.gridLocked; litColor: Theme.warn; tip: "Lock the grid: re-analysis will not change it"; onClicked: AppController.gridEdit(deck.dc.deck, "lock", deck.dc.gridLocked ? 0 : 1) }
            DjButton { implicitHeight: 26; icon: "refresh"; text: "RESET"; tip: "Back to the analyzed grid"; onClicked: AppController.gridEdit(deck.dc.deck, "reset", 0) }
            Item { Layout.fillWidth: true }
        }

        // --- Hotcues (the grid editor takes their place while GRID is on) ------
        RowLayout {
            Layout.fillWidth: true
            visible: !deck.gridEditing && deck.showHotcues
            spacing: 3
            Repeater {
                model: 8
                DjButton {
                    id: pad
                    required property int index
                    readonly property var cue: {
                        for (var i = 0; i < deck.hotcues.length; i++)
                            if (deck.hotcues[i].slot === index)
                                return deck.hotcues[i]
                        return null
                    }
                    Layout.fillWidth: true
                    implicitHeight: 26
                    text: (index + 1) + (cue && cue.loop ? " ↻" : "")
                    fontSize: Theme.fontNormal
                    target: deck.t("hotcue." + (index + 1))
                    lit: cue !== null
                    litColor: cue ? cue.color : Theme.accent
                    tip: cue ? "Hotcue " + (index + 1) + ": jump here; right-click deletes" : "Hotcue " + (index + 1) + ": set a cue point (or save the running loop) here"
                    onRightClicked: deck.tap("hotcue_delete." + (index + 1))
                }
            }
        }
    }
}
